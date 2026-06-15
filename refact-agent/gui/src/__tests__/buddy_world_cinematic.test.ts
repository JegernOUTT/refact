import { describe, expect, it, vi } from "vitest";
import {
  BUDDY_DREAM_KINDS,
  drawBuddyDreamFrame,
  pickBuddyDream,
} from "../features/Buddy/buddyDreams";
import { drawBuddyWorldForeground } from "../features/Buddy/buddyWorldDrawForeground";
import {
  drawLanternGlowPools,
  drawPondReflection,
} from "../features/Buddy/buddyWorldDrawDiorama";
import { chooseBuddyWorldIntent } from "../features/Buddy/buddyWorldDirector";
import { buildBuddyWorldState } from "../features/Buddy/buddyWorldModel";
import { PALETTES } from "../features/Buddy/constants";
import type { DrawBuddyWorldBaseArgs } from "../features/Buddy/buddyWorldDrawHelpers";

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
    globalCompositeOperation: "source-over",
    lineWidth: 1,
    lineCap: "butt",
    save: vi.fn(() => ops.push("save")),
    restore: vi.fn(() => ops.push("restore")),
    beginPath: vi.fn(() => ops.push("beginPath")),
    clearRect: vi.fn((x: number, y: number, w: number, h: number) =>
      push("clearRect", [x, y, w, h]),
    ),
    setTransform: vi.fn(),
    arc: vi.fn((x: number, y: number, r: number) => push("arc", [x, y, r])),
    ellipse: vi.fn((x: number, y: number, rx: number, ry: number) =>
      push("ellipse", [x, y, rx, ry]),
    ),
    moveTo: vi.fn((x: number, y: number) => push("moveTo", [x, y])),
    lineTo: vi.fn((x: number, y: number) => push("lineTo", [x, y])),
    bezierCurveTo: vi.fn(
      (
        c1x: number,
        c1y: number,
        c2x: number,
        c2y: number,
        x: number,
        y: number,
      ) => push("bezier", [c1x, c1y, c2x, c2y, x, y]),
    ),
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
  now: Date,
  overrides: Partial<DrawBuddyWorldBaseArgs> = {},
): DrawBuddyWorldBaseArgs {
  return {
    ctx: recording.ctx,
    world: buildBuddyWorldState({
      now,
      pulse: null,
      pet: undefined,
      nowPlaying: null,
      activeQuest: null,
    }),
    palette: PALETTES[0],
    frame: 300,
    width: 720,
    height: 260,
    compact: false,
    reducedMotion: false,
    ...overrides,
  };
}

describe("buddy dreams", () => {
  it("picks deterministically and reaches every dreamscape", () => {
    const seen = new Set<string>();
    for (let seed = 0; seed < 200; seed += 1) {
      const kind = pickBuddyDream(seed);
      expect(pickBuddyDream(seed)).toBe(kind);
      seen.add(kind);
    }
    expect(seen).toEqual(new Set(BUDDY_DREAM_KINDS));
  });

  it("clears before painting every dream frame", () => {
    for (const kind of BUDDY_DREAM_KINDS) {
      const recording = recordingContext();
      drawBuddyDreamFrame(recording.ctx, kind, 120);
      expect(recording.ops[0]?.startsWith("clearRect")).toBe(true);
      expect(recording.ops.length).toBeGreaterThan(2);
    }
  });

  it("renders dream frames deterministically", () => {
    const first = recordingContext();
    const second = recordingContext();
    drawBuddyDreamFrame(first.ctx, "brick_stars", 77);
    drawBuddyDreamFrame(second.ctx, "brick_stars", 77);
    expect(first.ops).toEqual(second.ops);
  });

  it("stays finite with hostile frames", () => {
    const recording = recordingContext();
    drawBuddyDreamFrame(recording.ctx, "sky_flight", Number.NaN);
    const flat = recording.ops.join("|");
    expect(flat).not.toContain("NaN");
    expect(flat).not.toContain("Infinity");
  });
});

describe("foreground depth canvas", () => {
  const day = new Date("2024-01-01T14:00:00");

  it("clears first and stays deterministic", () => {
    const first = recordingContext();
    const second = recordingContext();
    drawBuddyWorldForeground(drawArgsFor(first, day));
    drawBuddyWorldForeground(drawArgsFor(second, day));
    expect(first.ops[0]?.startsWith("clearRect")).toBe(true);
    expect(first.ops).toEqual(second.ops);
  });

  it("keeps reduced motion lighter than standard", () => {
    const standard = recordingContext();
    const reduced = recordingContext();
    drawBuddyWorldForeground(drawArgsFor(standard, day));
    drawBuddyWorldForeground(
      drawArgsFor(reduced, day, { reducedMotion: true }),
    );
    expect(reduced.ops.length).toBeLessThan(standard.ops.length);
  });

  it("keeps the tuft clusters outside the buddy band", () => {
    const recording = recordingContext();
    drawBuddyWorldForeground(drawArgsFor(recording, day));
    const ellipseXs = recording.ops
      .filter((op) => op.startsWith("ellipse"))
      .map((op) => Number(op.split(":")[1]))
      .filter((x) => Number.isFinite(x));
    const groundShadowXs = ellipseXs.filter((x) =>
      [86.4, 216, 619.2].includes(x),
    );
    expect(groundShadowXs.length).toBeGreaterThanOrEqual(3);
    for (const x of groundShadowXs) {
      const percent = (x / 720) * 100;
      expect(percent < 33 || percent > 67).toBe(true);
    }
  });

  it("rains in front of the buddy for rainy worlds", () => {
    const clear = recordingContext();
    drawBuddyWorldForeground(drawArgsFor(clear, day));

    const rainyWorld = {
      ...buildBuddyWorldState({
        now: day,
        pulse: null,
        pet: undefined,
        nowPlaying: null,
        activeQuest: null,
      }),
      weather: "rain" as const,
    };
    const rainy = recordingContext();
    const rainyRepeat = recordingContext();
    drawBuddyWorldForeground(drawArgsFor(rainy, day, { world: rainyWorld }));
    drawBuddyWorldForeground(
      drawArgsFor(rainyRepeat, day, { world: rainyWorld }),
    );
    const reduced = recordingContext();
    drawBuddyWorldForeground(
      drawArgsFor(reduced, day, { world: rainyWorld, reducedMotion: true }),
    );

    const streakCount = rainy.fillRectStyles.filter(
      (style) => style === "#0284C7",
    ).length;
    const reducedCount = reduced.fillRectStyles.filter(
      (style) => style === "#0284C7",
    ).length;
    expect(streakCount).toBeGreaterThanOrEqual(5);
    expect(clear.fillRectStyles).not.toContain("#0284C7");
    expect(rainyRepeat.ops).toEqual(rainy.ops);
    expect(reducedCount).toBeLessThan(streakCount);
  });

  it("stays finite with hostile inputs", () => {
    const recording = recordingContext();
    drawBuddyWorldForeground(
      drawArgsFor(recording, day, {
        frame: Number.POSITIVE_INFINITY,
        width: Number.NaN,
        height: Number.NaN,
      }),
    );
    const flat = recording.ops.join("|");
    expect(flat).not.toContain("NaN");
    expect(flat).not.toContain("Infinity");
  });
});

describe("pond reflection", () => {
  const summerDay = new Date("2024-07-01T14:00:00");

  it("appears only when the actor is near the pond", () => {
    const near = recordingContext();
    drawPondReflection(drawArgsFor(near, summerDay, { actorXPercent: 38 }));
    expect(near.ops.length).toBeGreaterThan(0);

    const far = recordingContext();
    drawPondReflection(drawArgsFor(far, summerDay, { actorXPercent: 60 }));
    expect(far.ops).toHaveLength(0);

    const absent = recordingContext();
    drawPondReflection(drawArgsFor(absent, summerDay));
    expect(absent.ops).toHaveLength(0);
  });

  it("skips the frozen winter pond", () => {
    const recording = recordingContext();
    drawPondReflection(
      drawArgsFor(recording, new Date("2024-01-01T14:00:00"), {
        actorXPercent: 38,
      }),
    );
    expect(recording.ops).toHaveLength(0);
  });
});

describe("lantern glow pools", () => {
  const night = new Date("2024-07-01T23:00:00");

  it("only glows in the evening and night", () => {
    const dayRecording = recordingContext();
    drawLanternGlowPools(
      drawArgsFor(dayRecording, new Date("2024-07-01T14:00:00")),
      3,
    );
    expect(dayRecording.ops).toHaveLength(0);

    const nightRecording = recordingContext();
    drawLanternGlowPools(drawArgsFor(nightRecording, night), 3);
    expect(nightRecording.ops.length).toBeGreaterThan(0);
  });

  it("respects the lit count override and avoids amber fill rects", () => {
    const twoLit = recordingContext();
    drawLanternGlowPools(drawArgsFor(twoLit, night), 2);
    const pools = twoLit.ops.filter(
      (op) => op.startsWith("fill:") && op.includes("#FBBF24"),
    );
    expect(pools).toHaveLength(2);
    expect(twoLit.fillRectStyles).toHaveLength(0);

    const doused = recordingContext();
    drawLanternGlowPools(drawArgsFor(doused, night), 0);
    expect(doused.ops).toHaveLength(0);
  });
});

describe("peek bush intent", () => {
  it("ducks behind the bush as a deep idle fallback", () => {
    const world = buildBuddyWorldState({
      now: new Date("2024-07-01T14:00:00"),
      pulse: null,
      pet: undefined,
      nowPlaying: null,
      activeQuest: null,
    });
    const intent = chooseBuddyWorldIntent({
      world,
      previousIntent: null,
      nowMs: 1_000,
      activeSpeechVisible: false,
      showcaseActive: false,
      localReactionVisible: false,
      reducedMotion: false,
      recentIntentKinds: [
        "wander_curiously",
        "watch_observatory",
        "nap_under_tree",
        "build_cairn",
        "paint_meadow",
        "picnic_snack",
        "spin_top",
        "fish_at_pond",
        "visit_pond",
        "tend_garden",
        "chase_butterfly",
        "watch_birds",
        "catch_fireflies",
        "smell_flowers",
        "collect_leaves",
        "morning_stretch",
      ],
    });
    expect(intent?.kind).toBe("peek_bush");
    expect(intent?.speech).toBeNull();
    expect(intent?.targetX).toBe(33);
  });

  it("never peeks during winter", () => {
    const world = buildBuddyWorldState({
      now: new Date("2024-01-01T14:00:00"),
      pulse: null,
      pet: undefined,
      nowPlaying: null,
      activeQuest: null,
    });
    const intent = chooseBuddyWorldIntent({
      world,
      previousIntent: null,
      nowMs: 1_000,
      activeSpeechVisible: false,
      showcaseActive: false,
      localReactionVisible: false,
      reducedMotion: false,
      recentIntentKinds: [
        "play_in_snow",
        "wander_curiously",
        "watch_observatory",
      ],
    });
    expect(intent?.kind ?? null).not.toBe("peek_bush");
  });
});
