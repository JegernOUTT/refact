import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { useBuddyWorldArcs } from "../features/Buddy/hooks/useBuddyWorldArcs";
import { createBuddySpeechMemory } from "../features/Buddy/buddySpeech";
import { buildBuddyWorldState } from "../features/Buddy/buddyWorldModel";
import {
  BUDDY_WORLD_ARC_DEFS,
  advanceBuddyArcRun,
  buddyArcLanternLitCount,
  chooseBuddyWorldArc,
  createBuddyArcRun,
  currentBuddyArcStep,
  seasonFirstMemoKey,
  type BuddyWorldArcKind,
  type ChooseBuddyWorldArcArgs,
} from "../features/Buddy/buddyWorldArcs";
import {
  BUDDY_WORLD_MEMOS_KEY,
  buddyWorldDayKey,
  createEmptyBuddyWorldMemos,
  readBuddyWorldMemos,
  writeBuddyWorldMemos,
} from "../features/Buddy/buddyWorldMemos";

const ARC_KINDS = Object.keys(BUDDY_WORLD_ARC_DEFS) as BuddyWorldArcKind[];

function chooseArgs(
  overrides: Partial<ChooseBuddyWorldArcArgs> = {},
): ChooseBuddyWorldArcArgs {
  return {
    previousPhase: "day",
    phase: "day",
    previousWeather: "clear",
    weather: "clear",
    layers: [],
    memos: createEmptyBuddyWorldMemos(),
    dayKey: "2026-06-12",
    year: 2026,
    busy: false,
    ...overrides,
  };
}

function fakeStorage(initial: Record<string, string> = {}): Storage {
  const data = new Map(Object.entries(initial));
  return {
    get length() {
      return data.size;
    },
    clear: () => data.clear(),
    getItem: (key: string) => data.get(key) ?? null,
    key: (index: number) => [...data.keys()][index] ?? null,
    removeItem: (key: string) => void data.delete(key),
    setItem: (key: string, value: string) => void data.set(key, value),
  };
}

describe("buddy world arc definitions", () => {
  it("keeps every step inside the safe travel band", () => {
    for (const kind of ARC_KINDS) {
      const def = BUDDY_WORLD_ARC_DEFS[kind];
      const steps = [
        ...def.steps,
        ...(def.finale ? [def.finale.clear, def.finale.stormy] : []),
      ];
      for (const step of steps) {
        expect(step.targetX).toBeGreaterThanOrEqual(33);
        expect(step.targetX).toBeLessThanOrEqual(67);
        expect(step.targetY).toBeGreaterThanOrEqual(58);
        expect(step.targetY).toBeLessThanOrEqual(84);
        expect(step.depthScale).toBeGreaterThanOrEqual(0.7);
        expect(step.depthScale).toBeLessThanOrEqual(1.2);
        expect(step.durationMs).toBeGreaterThan(2_000);
        expect(step.beats.length).toBeGreaterThan(0);
        for (const beat of step.beats) {
          expect(beat.atMs).toBeGreaterThanOrEqual(0);
          expect(beat.atMs).toBeLessThan(step.durationMs);
          expect(beat.lines.length).toBeGreaterThan(0);
          for (const line of beat.lines) {
            expect(line("Mochi").length).toBeGreaterThan(0);
          }
        }
      }
    }
  });

  it("lights the evening lanterns one at a time", () => {
    const steps = BUDDY_WORLD_ARC_DEFS.evening_lanterns.steps;
    expect(steps.map((step) => step.lanternLitCount)).toEqual([1, 2, 3, 3]);
  });
});

describe("chooseBuddyWorldArc", () => {
  it("starts the storm story when a storm rolls in", () => {
    expect(
      chooseBuddyWorldArc(
        chooseArgs({ previousWeather: "clear", weather: "storm" }),
      ),
    ).toBe("storm_story");
  });

  it("does not restart the storm story on the same day", () => {
    const memos = createEmptyBuddyWorldMemos();
    memos.lastArcDates.storm_story = "2026-06-12";
    expect(
      chooseBuddyWorldArc(
        chooseArgs({ previousWeather: "clear", weather: "storm", memos }),
      ),
    ).toBeNull();
  });

  it("starts the morning ritual on a phase transition into morning", () => {
    expect(
      chooseBuddyWorldArc(
        chooseArgs({ previousPhase: "night", phase: "morning" }),
      ),
    ).toBe("morning_ritual");
  });

  it("starts the lantern walk on a transition into evening", () => {
    expect(
      chooseBuddyWorldArc(
        chooseArgs({ previousPhase: "day", phase: "evening" }),
      ),
    ).toBe("evening_lanterns");
  });

  it("requires an observed transition for daily arcs", () => {
    expect(
      chooseBuddyWorldArc(
        chooseArgs({ previousPhase: null, phase: "morning" }),
      ),
    ).toBeNull();
    expect(
      chooseBuddyWorldArc(
        chooseArgs({ previousPhase: "morning", phase: "morning" }),
      ),
    ).toBeNull();
  });

  it("runs each daily arc only once per day", () => {
    const memos = createEmptyBuddyWorldMemos();
    memos.lastArcDates.morning_ritual = "2026-06-12";
    expect(
      chooseBuddyWorldArc(
        chooseArgs({ previousPhase: "night", phase: "morning", memos }),
      ),
    ).toBeNull();
    expect(
      chooseBuddyWorldArc(
        chooseArgs({
          previousPhase: "night",
          phase: "morning",
          memos,
          dayKey: "2026-06-13",
        }),
      ),
    ).toBe("morning_ritual");
  });

  it("suppresses non-storm arcs during a storm", () => {
    expect(
      chooseBuddyWorldArc(
        chooseArgs({
          previousPhase: "night",
          phase: "morning",
          previousWeather: "storm",
          weather: "storm",
        }),
      ),
    ).toBeNull();
  });

  it("fires each season first once per year from its layer", () => {
    expect(chooseBuddyWorldArc(chooseArgs({ layers: ["season_snow"] }))).toBe(
      "first_snowflake",
    );
    const memos = createEmptyBuddyWorldMemos();
    memos.seasonFirstsSeen = [seasonFirstMemoKey("first_snowflake", 2026)];
    expect(
      chooseBuddyWorldArc(chooseArgs({ layers: ["season_snow"], memos })),
    ).toBeNull();
    expect(
      chooseBuddyWorldArc(
        chooseArgs({ layers: ["season_snow"], memos, year: 2027 }),
      ),
    ).toBe("first_snowflake");
  });

  it("maps every season layer to its first-of-season arc", () => {
    expect(chooseBuddyWorldArc(chooseArgs({ layers: ["season_petals"] }))).toBe(
      "first_petal",
    );
    expect(chooseBuddyWorldArc(chooseArgs({ layers: ["fireflies"] }))).toBe(
      "first_firefly",
    );
    expect(chooseBuddyWorldArc(chooseArgs({ layers: ["season_leaves"] }))).toBe(
      "first_red_leaf",
    );
  });

  it("never starts an arc while busy", () => {
    expect(
      chooseBuddyWorldArc(
        chooseArgs({
          previousPhase: "night",
          phase: "morning",
          busy: true,
        }),
      ),
    ).toBeNull();
  });
});

describe("arc run advancement", () => {
  it("walks the morning ritual steps in order and ends", () => {
    const ids: string[] = [];
    let run: ReturnType<typeof advanceBuddyArcRun> = createBuddyArcRun(
      "morning_ritual",
      0,
    );
    let guard = 0;
    while (run !== null && guard < 10) {
      guard += 1;
      const step = currentBuddyArcStep(run);
      if (!step) break;
      ids.push(step.id);
      run = advanceBuddyArcRun(
        run,
        run.stepStartedAtMs + step.durationMs,
        "clear",
      );
    }
    expect(run).toBeNull();
    expect(ids).toEqual([
      "wake_stretch",
      "water_garden",
      "check_mailbox",
      "sun_salute",
    ]);
  });

  it("holds the current step until its duration elapses", () => {
    const run = createBuddyArcRun("morning_ritual", 1_000);
    expect(advanceBuddyArcRun(run, 2_000, "clear")).toBe(run);
  });

  it("ends the storm story with a rainbow walk when the sky clears", () => {
    let run = createBuddyArcRun("storm_story", 0);
    for (const step of BUDDY_WORLD_ARC_DEFS.storm_story.steps) {
      const next = advanceBuddyArcRun(
        run,
        run.stepStartedAtMs + step.durationMs,
        "clear",
      );
      expect(next).not.toBeNull();
      if (next) run = next;
    }
    expect(run.finale).toBe("clear");
    expect(currentBuddyArcStep(run)?.id).toBe("rainbow_walk");
    const finaleStep = currentBuddyArcStep(run);
    expect(
      advanceBuddyArcRun(
        run,
        run.stepStartedAtMs + (finaleStep?.durationMs ?? 0),
        "clear",
      ),
    ).toBeNull();
  });

  it("ends the storm story quietly while it still rains", () => {
    let run = createBuddyArcRun("storm_story", 0);
    for (const step of BUDDY_WORLD_ARC_DEFS.storm_story.steps) {
      const next = advanceBuddyArcRun(
        run,
        run.stepStartedAtMs + step.durationMs,
        "rain",
      );
      if (next) run = next;
    }
    expect(run.finale).toBe("stormy");
    expect(currentBuddyArcStep(run)?.id).toBe("quiet_end");
  });

  it("reports the lantern lit count for the active step", () => {
    let run = createBuddyArcRun("evening_lanterns", 0);
    const litCounts: (number | null)[] = [buddyArcLanternLitCount(run)];
    for (let index = 0; index < 3; index += 1) {
      const step = currentBuddyArcStep(run);
      const next = advanceBuddyArcRun(
        run,
        run.stepStartedAtMs + (step?.durationMs ?? 0),
        "clear",
      );
      if (!next) break;
      run = next;
      litCounts.push(buddyArcLanternLitCount(run));
    }
    expect(litCounts).toEqual([1, 2, 3, 3]);
    expect(buddyArcLanternLitCount(null)).toBeNull();
  });
});

describe("buddy world memos", () => {
  it("round-trips memos through storage", () => {
    const storage = fakeStorage();
    writeBuddyWorldMemos(
      { lastArcDates: { morning_ritual: "2026-06-12" } },
      storage,
    );
    writeBuddyWorldMemos(
      { seasonFirstsSeen: ["first_snowflake:2026"], shiroIntroSeen: true },
      storage,
    );
    const memos = readBuddyWorldMemos(storage);
    expect(memos.lastArcDates.morning_ritual).toBe("2026-06-12");
    expect(memos.seasonFirstsSeen).toEqual(["first_snowflake:2026"]);
    expect(memos.shiroIntroSeen).toBe(true);
  });

  it("dedupes season firsts on write", () => {
    const storage = fakeStorage();
    writeBuddyWorldMemos({ seasonFirstsSeen: ["a"] }, storage);
    writeBuddyWorldMemos({ seasonFirstsSeen: ["a", "b"] }, storage);
    expect(readBuddyWorldMemos(storage).seasonFirstsSeen).toEqual(["a", "b"]);
  });

  it("survives corrupted storage payloads", () => {
    const storage = fakeStorage({ [BUDDY_WORLD_MEMOS_KEY]: "{not json" });
    expect(readBuddyWorldMemos(storage)).toEqual(createEmptyBuddyWorldMemos());
    const wrongShape = fakeStorage({
      [BUDDY_WORLD_MEMOS_KEY]: JSON.stringify({
        lastArcDates: { x: 7 },
        seasonFirstsSeen: [1, "ok"],
        shiroIntroSeen: "yes",
      }),
    });
    const memos = readBuddyWorldMemos(wrongShape);
    expect(memos.lastArcDates).toEqual({});
    expect(memos.seasonFirstsSeen).toEqual(["ok"]);
    expect(memos.shiroIntroSeen).toBe(false);
  });

  it("formats day keys as zero-padded dates", () => {
    expect(buddyWorldDayKey(new Date(2026, 0, 5))).toBe("2026-01-05");
    expect(buddyWorldDayKey(new Date(2026, 11, 31))).toBe("2026-12-31");
  });
});

describe("useBuddyWorldArcs line ownership", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-07-01T23:00:00"));
    window.localStorage.removeItem(BUDDY_WORLD_MEMOS_KEY);
    writeBuddyWorldMemos({
      seasonFirstsSeen: [
        "first_firefly:2024",
        "first_petal:2024",
        "first_red_leaf:2024",
        "first_snowflake:2024",
      ],
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    window.localStorage.removeItem(BUDDY_WORLD_MEMOS_KEY);
  });

  function worldAt(time: string) {
    return buildBuddyWorldState({
      now: new Date(time),
      pulse: null,
      pet: undefined,
      nowPlaying: null,
      activeQuest: null,
    });
  }

  it("clears the arc line on every step transition", () => {
    const speechMemory = createBuddySpeechMemory();
    const baseArgs = {
      name: "Mochi",
      busy: false,
      showcaseActive: false,
      showcaseIsRuntime: false,
      reducedMotion: false,
      speechMemory,
    };
    const { result, rerender } = renderHook(
      ({ world }) => useBuddyWorldArcs({ ...baseArgs, world }),
      { initialProps: { world: worldAt("2024-07-01T23:00:00") } },
    );
    expect(result.current.arcRun).toBeNull();

    rerender({ world: worldAt("2024-07-01T09:30:00") });
    expect(result.current.arcRun?.kind).toBe("morning_ritual");
    expect(result.current.arcStep?.id).toBe("wake_stretch");
    expect(result.current.arcLine).toBeNull();

    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(result.current.arcLine).not.toBeNull();

    act(() => {
      vi.advanceTimersByTime(5_000);
    });
    expect(result.current.arcStep?.id).toBe("water_garden");
    expect(result.current.arcLine).toBeNull();

    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    expect(result.current.arcLine).not.toBeNull();
  });

  it("cancels the arc and its line when busy", () => {
    const speechMemory = createBuddySpeechMemory();
    const baseArgs = {
      name: "Mochi",
      showcaseActive: false,
      showcaseIsRuntime: false,
      reducedMotion: false,
      speechMemory,
    };
    const { result, rerender } = renderHook(
      ({ world, busy }) => useBuddyWorldArcs({ ...baseArgs, world, busy }),
      {
        initialProps: {
          world: worldAt("2024-07-01T23:00:00"),
          busy: false,
        },
      },
    );
    rerender({ world: worldAt("2024-07-01T09:30:00"), busy: false });
    act(() => {
      vi.advanceTimersByTime(600);
    });
    expect(result.current.arcRun).not.toBeNull();
    expect(result.current.arcLine).not.toBeNull();

    rerender({ world: worldAt("2024-07-01T09:30:00"), busy: true });
    expect(result.current.arcRun).toBeNull();
    expect(result.current.arcLine).toBeNull();
  });
});
