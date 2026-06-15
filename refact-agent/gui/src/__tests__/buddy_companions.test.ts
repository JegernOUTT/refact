import { describe, expect, it, vi } from "vitest";
import {
  KURO_PERCH,
  SHIRO_HIDE_SPOT,
  companionPositionAt,
  companionSeededUnit,
  companionSeedFromText,
  createShiroState,
  kuroCompanion,
  kuroDayActive,
  shiroCompanion,
  sootCompanions,
  stepKuroState,
  stepShiroState,
  type BuddyWorldCompanion,
  type ShiroContext,
} from "../features/Buddy/buddyCompanions";
import {
  drawBuddyWorldCompanions,
  drawHomeWindowGlow,
} from "../features/Buddy/buddyWorldDrawCompanions";
import { buildBuddyWorldState } from "../features/Buddy/buddyWorldModel";
import { PALETTES } from "../features/Buddy/constants";
import type { DrawBuddyWorldBaseArgs } from "../features/Buddy/buddyWorldDrawHelpers";

function shiroContext(overrides: Partial<ShiroContext> = {}): ShiroContext {
  return {
    buddyX: 50,
    buddyY: 78,
    buddyPose: "idle",
    longActionActive: false,
    sleeping: false,
    storm: false,
    nowMs: 10_000,
    random: () => 0.1,
    ...overrides,
  };
}

interface RecordingContext {
  ctx: CanvasRenderingContext2D;
  ops: string[];
  fillRectStyles: string[];
}

function recordingContext(): RecordingContext {
  const ops: string[] = [];
  const fillRectStyles: string[] = [];
  let fillStyle = "";
  let strokeStyle = "";
  const push = (name: string, values: number[]) => {
    ops.push(`${name}:${values.map((value) => value.toFixed(2)).join(":")}`);
  };
  const ctx = {
    set fillStyle(value: string) {
      fillStyle = value;
    },
    get fillStyle() {
      return fillStyle;
    },
    set strokeStyle(value: string) {
      strokeStyle = value;
    },
    get strokeStyle() {
      return strokeStyle;
    },
    globalAlpha: 1,
    lineWidth: 1,
    lineCap: "butt",
    save: vi.fn(() => ops.push("save")),
    restore: vi.fn(() => ops.push("restore")),
    beginPath: vi.fn(() => ops.push("beginPath")),
    arc: vi.fn((x: number, y: number, r: number) => push("arc", [x, y, r])),
    ellipse: vi.fn((x: number, y: number, rx: number, ry: number) =>
      push("ellipse", [x, y, rx, ry]),
    ),
    moveTo: vi.fn((x: number, y: number) => push("moveTo", [x, y])),
    lineTo: vi.fn((x: number, y: number) => push("lineTo", [x, y])),
    fill: vi.fn(() => ops.push(`fill:${fillStyle}`)),
    stroke: vi.fn(() => ops.push(`stroke:${strokeStyle}`)),
    fillRect: vi.fn((x: number, y: number, w: number, h: number) => {
      push("fillRect", [x, y, w, h]);
      fillRectStyles.push(fillStyle);
    }),
  } as unknown as CanvasRenderingContext2D;
  return { ctx, ops, fillRectStyles };
}

function drawArgsFor(
  recording: RecordingContext,
  worldOverrides: Partial<Parameters<typeof buildBuddyWorldState>[0]> = {},
): DrawBuddyWorldBaseArgs {
  return {
    ctx: recording.ctx,
    world: buildBuddyWorldState({
      now: new Date("2024-01-01T21:30:00"),
      pulse: null,
      pet: undefined,
      nowPlaying: null,
      activeQuest: null,
      ...worldOverrides,
    }),
    palette: PALETTES[0],
    frame: 240,
    width: 720,
    height: 260,
    compact: false,
    reducedMotion: false,
  };
}

function sampleCompanions(): BuddyWorldCompanion[] {
  return [
    {
      id: "shiro",
      kind: "shiro",
      fromX: 40,
      fromY: 78,
      toX: 55,
      toY: 80,
      moveStartMs: 0,
      moveDurationMs: 2_000,
      scale: 0.38,
      facing: 1,
      pose: "idle",
      seed: 71,
    },
    {
      id: "soot-roots",
      kind: "soot",
      fromX: 29.5,
      fromY: 80,
      toX: 29.5,
      toY: 80,
      moveStartMs: 0,
      moveDurationMs: 1,
      scale: 1,
      facing: 1,
      pose: "idle",
      seed: 17,
    },
    {
      id: "kuro",
      kind: "kuro",
      fromX: 31,
      fromY: 61.5,
      toX: 31,
      toY: 61.5,
      moveStartMs: 0,
      moveDurationMs: 1,
      scale: 1,
      facing: 1,
      pose: "perch",
      seed: 137,
    },
  ];
}

describe("companion movement math", () => {
  it("eases between waypoints and clamps progress", () => {
    const companion = sampleCompanions()[0];
    const start = companionPositionAt(companion, 0);
    const mid = companionPositionAt(companion, 1_000);
    const end = companionPositionAt(companion, 9_999);
    expect(start.x).toBeCloseTo(40, 5);
    expect(end.x).toBeCloseTo(55, 5);
    expect(mid.x).toBeGreaterThan(40);
    expect(mid.x).toBeLessThan(55);
  });

  it("stays finite with hostile inputs", () => {
    const position = companionPositionAt(
      {
        fromX: Number.NaN,
        fromY: Number.POSITIVE_INFINITY,
        toX: Number.NEGATIVE_INFINITY,
        toY: Number.NaN,
        moveStartMs: Number.NaN,
        moveDurationMs: 0,
      },
      Number.NaN,
    );
    expect(Number.isFinite(position.x)).toBe(true);
    expect(Number.isFinite(position.y)).toBe(true);
  });

  it("hashes deterministically", () => {
    expect(companionSeedFromText("2026-06-12")).toBe(
      companionSeedFromText("2026-06-12"),
    );
    const unit = companionSeededUnit(companionSeedFromText("x"), 3);
    expect(unit).toBeGreaterThanOrEqual(0);
    expect(unit).toBeLessThanOrEqual(1);
  });
});

describe("shiro state machine", () => {
  it("hides behind the great tree during storms", () => {
    const state = stepShiroState(
      createShiroState(0, 50, 78),
      shiroContext({ storm: true }),
    );
    expect(state.mode).toBe("hide");
    expect(state.toX).toBeCloseTo(SHIRO_HIDE_SPOT.x, 5);
    expect(shiroCompanion(state).pose).toBe("peek");
  });

  it("piles up for naps when the buddy sleeps", () => {
    const state = stepShiroState(
      createShiroState(0, 50, 78),
      shiroContext({ sleeping: true }),
    );
    expect(state.mode).toBe("nap_pile");
    expect(state.toX).toBeCloseTo(55, 5);
    expect(shiroCompanion(state).pose).toBe("sleep");
  });

  it("sits to watch long actions and keeps its side", () => {
    const first = stepShiroState(
      createShiroState(0, 50, 78),
      shiroContext({ longActionActive: true }),
    );
    expect(first.mode).toBe("watch");
    const second = stepShiroState(
      first,
      shiroContext({ longActionActive: true, nowMs: 12_500 }),
    );
    expect(second.watchSide).toBe(first.watchSide);
    expect(shiroCompanion(second).pose).toBe("sit");
  });

  it("mimics the buddy pose one tick late", () => {
    const state = stepShiroState(
      createShiroState(0, 50, 78),
      shiroContext({ random: () => 0.5, buddyPose: "sleepy" }),
    );
    expect(state.mode).toBe("mimic");
    expect(state.mimicPose).toBe("sleepy");
    expect(shiroCompanion(state).pose).toBe("sleep");
  });

  it("clamps targets within the world even with hostile context", () => {
    const state = stepShiroState(
      createShiroState(0, 50, 78),
      shiroContext({
        buddyX: Number.NaN,
        buddyY: Number.POSITIVE_INFINITY,
        random: () => 0.99,
      }),
    );
    expect(state.toX).toBeGreaterThanOrEqual(6);
    expect(state.toX).toBeLessThanOrEqual(94);
    expect(state.toY).toBeGreaterThanOrEqual(70);
    expect(state.toY).toBeLessThanOrEqual(86);
  });
});

describe("soot colony", () => {
  it("gathers three sprites in the evening", () => {
    const soot = sootCompanions({
      phase: "evening",
      weather: "clear",
      layers: ["campfire"],
      buddyX: 50,
      dayKey: "2026-06-12",
    });
    expect(soot).toHaveLength(3);
    expect(soot.every((sprite) => sprite.kind === "soot")).toBe(true);
  });

  it("hides from rain and storms", () => {
    for (const weather of ["rain", "storm"] as const) {
      expect(
        sootCompanions({
          phase: "night",
          weather,
          layers: [],
          buddyX: 50,
          dayKey: "2026-06-12",
        }),
      ).toHaveLength(0);
    }
  });

  it("scatters sprites the buddy rushes past", () => {
    const soot = sootCompanions({
      phase: "night",
      weather: "clear",
      layers: [],
      buddyX: 30,
      dayKey: "2026-06-12",
    });
    const roots = soot.find((sprite) => sprite.id === "soot-roots");
    expect(roots?.pose).toBe("flee");
    expect(roots?.toX).not.toBe(roots?.fromX);
  });

  it("is deterministic per day key", () => {
    const args = {
      phase: "day" as const,
      weather: "clear" as const,
      layers: [] as never[],
      buddyX: 50,
    };
    for (const dayKey of ["2026-06-12", "2026-06-13", "2026-06-14"]) {
      const first = sootCompanions({ ...args, dayKey });
      const second = sootCompanions({ ...args, dayKey });
      expect(first).toEqual(second);
      expect(first.length).toBeLessThanOrEqual(1);
    }
  });
});

describe("kuro the crow", () => {
  it("only visits on seeded autumn days", () => {
    expect(kuroDayActive("2026-06-12", "summer")).toBe(false);
    const autumnDays = Array.from({ length: 20 }, (_, index) =>
      kuroDayActive(`2026-10-${String(index + 1).padStart(2, "0")}`, "autumn"),
    );
    expect(autumnDays.some(Boolean)).toBe(true);
    expect(autumnDays.every(Boolean)).toBe(false);
  });

  it("perches during acorn gathering and flees when the buddy closes in", () => {
    const state = { mode: "away" as const, sinceMs: 0 };
    const perched = stepKuroState(state, {
      active: true,
      gatherActive: true,
      buddyX: 60,
      nowMs: 1_000,
    });
    expect(perched.state.mode).toBe("perch");
    expect(kuroCompanion(perched.state)?.pose).toBe("perch");

    const fled = stepKuroState(perched.state, {
      active: true,
      gatherActive: true,
      buddyX: KURO_PERCH.x + 2,
      nowMs: 2_000,
    });
    expect(fled.state.mode).toBe("flee");
    expect(fled.fledNow).toBe(true);
    expect(kuroCompanion(fled.state)?.pose).toBe("flee");

    const gone = stepKuroState(fled.state, {
      active: true,
      gatherActive: false,
      buddyX: 50,
      nowMs: 4_000,
    });
    expect(gone.state.mode).toBe("away");
    expect(kuroCompanion(gone.state)).toBeNull();
  });

  it("stays away outside active days", () => {
    const result = stepKuroState(
      { mode: "perch", sinceMs: 0 },
      { active: false, gatherActive: true, buddyX: 60, nowMs: 1_000 },
    );
    expect(result.state.mode).toBe("away");
  });
});

describe("companion drawing", () => {
  it("draws deterministically for the same inputs", () => {
    const first = recordingContext();
    const second = recordingContext();
    drawBuddyWorldCompanions(drawArgsFor(first), sampleCompanions(), 1_000);
    drawBuddyWorldCompanions(drawArgsFor(second), sampleCompanions(), 1_000);
    expect(first.ops).toEqual(second.ops);
    expect(first.ops.length).toBeGreaterThan(0);
  });

  it("stays finite with hostile companion inputs", () => {
    const recording = recordingContext();
    drawBuddyWorldCompanions(
      drawArgsFor(recording),
      [
        {
          id: "bad",
          kind: "shiro",
          fromX: Number.NaN,
          fromY: Number.POSITIVE_INFINITY,
          toX: Number.NEGATIVE_INFINITY,
          toY: Number.NaN,
          moveStartMs: Number.NaN,
          moveDurationMs: Number.NaN,
          scale: Number.NaN,
          facing: 1,
          pose: "idle",
          seed: Number.NaN,
        },
      ],
      Number.NaN,
    );
    const flat = recording.ops.join("|");
    expect(flat).not.toContain("NaN");
    expect(flat).not.toContain("Infinity");
  });

  it("adds no amber fill rects to the night scene from companions", () => {
    const recording = recordingContext();
    drawBuddyWorldCompanions(drawArgsFor(recording), sampleCompanions(), 500);
    expect(recording.fillRectStyles).not.toContain("#F59E0B");
  });

  it("lights the home window only in the evening and at night", () => {
    const nightRecording = recordingContext();
    drawHomeWindowGlow(drawArgsFor(nightRecording));
    expect(nightRecording.fillRectStyles).toContain("#FDE68A");
    expect(nightRecording.fillRectStyles).not.toContain("#F59E0B");

    const dayRecording = recordingContext();
    drawHomeWindowGlow({
      ...drawArgsFor(dayRecording),
      world: buildBuddyWorldState({
        now: new Date("2024-01-01T14:00:00"),
        pulse: null,
        pet: undefined,
        nowPlaying: null,
        activeQuest: null,
      }),
    });
    expect(dayRecording.ops).toHaveLength(0);
  });

  it("draws the window glow deterministically per frame", () => {
    const first = recordingContext();
    const second = recordingContext();
    drawHomeWindowGlow(drawArgsFor(first));
    drawHomeWindowGlow(drawArgsFor(second));
    expect(first.ops).toEqual(second.ops);
  });
});
