import { describe, expect, it, vi } from "vitest";
import {
  advanceBuddyShowcasePhase,
  BUDDY_SHOWCASE_IDLE_COOLDOWN_MS,
  BUDDY_SHOWCASE_INITIAL_GRACE_MS,
  BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS,
  BUDDY_SHOWCASE_PHASE_DURATIONS_MS,
  chooseBuddyShowcase,
  createBuddyShowcaseRun,
  hasBuddyShowcaseRuntimeTrigger,
  type BuddyShowcaseTargetCandidate,
} from "../features/Buddy/buddyShowcase";
import { drawShowcaseEvent } from "../features/Buddy/buddyShowcaseDraw";
import { buildBuddyWorldState } from "../features/Buddy/buddyWorldModel";
import { PALETTES } from "../features/Buddy/constants";
import type {
  BuddyPetState,
  BuddyPulse,
  BuddyRuntimeEvent,
  BuddyShowcaseRun,
} from "../features/Buddy/types";

const MEMORY_TARGET: BuddyShowcaseTargetCandidate = {
  id: "memory",
  x: 33,
  y: 52,
  label: "Memory fireflies",
  sprite: "memory_fireflies",
};

const OBSERVATORY_TARGET: BuddyShowcaseTargetCandidate = {
  id: "providers",
  x: 72,
  y: 67,
  label: "Model observatory",
  sprite: "observatory",
};

type MockCanvasContext = Pick<
  CanvasRenderingContext2D,
  | "arc"
  | "beginPath"
  | "bezierCurveTo"
  | "clearRect"
  | "closePath"
  | "createLinearGradient"
  | "ellipse"
  | "fill"
  | "fillRect"
  | "fillText"
  | "lineTo"
  | "moveTo"
  | "restore"
  | "save"
  | "stroke"
> &
  Partial<CanvasRenderingContext2D>;

type RecordedCanvasContext = CanvasRenderingContext2D & {
  alphaWrites: number[];
  drawOps: string[];
};

function makeCanvasContext(): RecordedCanvasContext {
  const gradient = { addColorStop: vi.fn() } as unknown as CanvasGradient;
  const alphaWrites: number[] = [];
  const drawOps: string[] = [];
  let globalAlphaValue = 1;
  let fillStyleValue: CanvasRenderingContext2D["fillStyle"] = "#000000";
  let strokeStyleValue: CanvasRenderingContext2D["strokeStyle"] = "#000000";
  const formatNumber = (value: number) => value.toFixed(3);
  const ctx: MockCanvasContext & {
    alphaWrites: number[];
    drawOps: string[];
  } = {
    alphaWrites,
    drawOps,
    arc: vi.fn(
      (
        x: number,
        y: number,
        radius: number,
        startAngle: number,
        endAngle: number,
      ) => {
        drawOps.push(
          `arc:${formatNumber(x)}:${formatNumber(y)}:${formatNumber(
            radius,
          )}:${formatNumber(startAngle)}:${formatNumber(endAngle)}`,
        );
      },
    ),
    beginPath: vi.fn(() => drawOps.push("beginPath")),
    bezierCurveTo: vi.fn(
      (
        cp1x: number,
        cp1y: number,
        cp2x: number,
        cp2y: number,
        x: number,
        y: number,
      ) => {
        drawOps.push(
          `bezierCurveTo:${formatNumber(cp1x)}:${formatNumber(
            cp1y,
          )}:${formatNumber(cp2x)}:${formatNumber(cp2y)}:${formatNumber(
            x,
          )}:${formatNumber(y)}`,
        );
      },
    ),
    clearRect: vi.fn((x: number, y: number, width: number, height: number) => {
      drawOps.push(
        `clearRect:${formatNumber(x)}:${formatNumber(y)}:${formatNumber(
          width,
        )}:${formatNumber(height)}`,
      );
    }),
    closePath: vi.fn(() => drawOps.push("closePath")),
    createLinearGradient: vi.fn(
      (x0: number, y0: number, x1: number, y1: number) => {
        drawOps.push(
          `createLinearGradient:${formatNumber(x0)}:${formatNumber(
            y0,
          )}:${formatNumber(x1)}:${formatNumber(y1)}`,
        );
        return gradient;
      },
    ),
    ellipse: vi.fn(
      (
        x: number,
        y: number,
        radiusX: number,
        radiusY: number,
        rotation: number,
        startAngle: number,
        endAngle: number,
      ) => {
        drawOps.push(
          `ellipse:${formatNumber(x)}:${formatNumber(y)}:${formatNumber(
            radiusX,
          )}:${formatNumber(radiusY)}:${formatNumber(rotation)}:${formatNumber(
            startAngle,
          )}:${formatNumber(endAngle)}`,
        );
      },
    ),
    fill: vi.fn(() =>
      drawOps.push(`fill:${String(fillStyleValue)}:${globalAlphaValue}`),
    ),
    fillRect: vi.fn((x: number, y: number, width: number, height: number) => {
      drawOps.push(
        `fillRect:${formatNumber(x)}:${formatNumber(y)}:${formatNumber(
          width,
        )}:${formatNumber(height)}:${String(
          fillStyleValue,
        )}:${globalAlphaValue}`,
      );
    }),
    fillText: vi.fn((text: string, x: number, y: number) => {
      drawOps.push(
        `fillText:${text}:${formatNumber(x)}:${formatNumber(y)}:${String(
          fillStyleValue,
        )}:${globalAlphaValue}`,
      );
    }),
    lineTo: vi.fn((x: number, y: number) => {
      drawOps.push(`lineTo:${formatNumber(x)}:${formatNumber(y)}`);
    }),
    moveTo: vi.fn((x: number, y: number) => {
      drawOps.push(`moveTo:${formatNumber(x)}:${formatNumber(y)}`);
    }),
    restore: vi.fn(() => drawOps.push("restore")),
    save: vi.fn(() => drawOps.push("save")),
    stroke: vi.fn(() =>
      drawOps.push(`stroke:${String(strokeStyleValue)}:${globalAlphaValue}`),
    ),
    font: "10px monospace",
    get fillStyle() {
      return fillStyleValue;
    },
    set fillStyle(value: CanvasRenderingContext2D["fillStyle"]) {
      fillStyleValue = value;
    },
    get globalAlpha() {
      return globalAlphaValue;
    },
    set globalAlpha(value: number) {
      alphaWrites.push(value);
      globalAlphaValue = value;
    },
    imageSmoothingEnabled: false,
    lineCap: "round" as CanvasLineCap,
    lineWidth: 1,
    get strokeStyle() {
      return strokeStyleValue;
    },
    set strokeStyle(value: CanvasRenderingContext2D["strokeStyle"]) {
      strokeStyleValue = value;
    },
    textAlign: "center" as CanvasTextAlign,
    textBaseline: "middle" as CanvasTextBaseline,
  };
  return ctx as RecordedCanvasContext;
}

function makePet(sleeping = false): BuddyPetState {
  return {
    needs: {
      hunger: 80,
      energy: 80,
      hygiene: 80,
      boredom: 10,
      affection: 80,
    },
    condition: {
      sleeping,
      hungry: false,
      sleepy: false,
      dirty: false,
      bored: false,
      lonely: false,
    },
    evolution: {
      care_score: 0,
      neglect_score: 0,
      open_seconds: 0,
      last_evolved_at: null,
    },
  };
}

function makeRuntimeEvent(
  overrides?: Partial<BuddyRuntimeEvent>,
): BuddyRuntimeEvent {
  return {
    id: "runtime-1",
    signal_type: "memory_extract",
    title: "Memory extracted",
    source: "test",
    status: "completed",
    priority: "normal",
    created_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

function makePulse(overrides?: Partial<BuddyPulse>): BuddyPulse {
  const pulse: BuddyPulse = {
    generated_at: "2024-01-01T00:00:00Z",
    tasks: { total: 3, stuck: 0, abandoned: 0, by_status: {} },
    trajectories: { total: 10, untitled: 0, oldest_age_days: 1 },
    memory: { total: 5, orphan: 0, stale_conflicts: 0 },
    providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 0 },
    mcp: { total: 4, failing: 0, auth_expiring: 0 },
    customization: { modes: 3, skills: 2, commands: 1, subagents: 0, hooks: 0 },
    diagnostics: { last_hour: 0, top_error_types: [] },
    git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
    worktrees: {
      total_registered: 3,
      total_discovered: 1,
      total: 4,
      clean: 2,
      dirty: 1,
      unknown: 0,
      stale: 1,
      conflicted: 0,
      shared: 1,
      abandoned_clean: 2,
      changed_files: 3,
      additions: 10,
      deletions: 2,
      missing_registry_paths: 1,
      unregistered_cache_dirs: 1,
      merged_branches: 2,
    },
  };
  return { ...pulse, ...overrides };
}

function makeWorld() {
  return buildBuddyWorldState({
    now: new Date("2024-01-01T23:00:00"),
    pulse: makePulse(),
    pet: makePet(),
    nowPlaying: null,
    activeQuest: null,
  });
}

function makeShowcaseRun(
  overrides?: Partial<BuddyShowcaseRun>,
): BuddyShowcaseRun {
  return {
    id: "showcase-test",
    kind: "memory_firefly_night",
    phase: "showcase",
    target: MEMORY_TARGET,
    pose: "meditate",
    speech: "Buddy gathers the memory fireflies into a soft night map.",
    seed: 12345,
    startedAtMs: 1_000,
    phaseStartedAtMs: 1_000,
    ...overrides,
  };
}

function expectAlphaWritesClamped(ctx: RecordedCanvasContext): void {
  expect(ctx.alphaWrites.length).toBeGreaterThan(0);
  expect(ctx.alphaWrites.every((alpha) => alpha >= 0 && alpha <= 1)).toBe(true);
}

function expectAlphaWritesFinite(ctx: RecordedCanvasContext): void {
  expect(ctx.alphaWrites.length).toBeGreaterThan(0);
  expect(ctx.alphaWrites.every((alpha) => Number.isFinite(alpha))).toBe(true);
}

function expectDrawOpsFinite(ctx: RecordedCanvasContext): void {
  const serializedOps = ctx.drawOps.join(":");
  const numericTokens = ctx.drawOps.flatMap((operation) =>
    operation
      .split(":")
      .map((token) => Number(token))
      .filter((value) => !Number.isNaN(value)),
  );
  expect(serializedOps).not.toMatch(/\b(?:NaN|Infinity|-Infinity)\b/);
  expect(numericTokens.length).toBeGreaterThan(0);
  expect(numericTokens.every((value) => Number.isFinite(value))).toBe(true);
}

describe("buddy showcase director", () => {
  it("draws showcase overlay events for both supported kinds", () => {
    const world = makeWorld();

    expect(() =>
      drawShowcaseEvent({
        ctx: makeCanvasContext(),
        run: makeShowcaseRun(),
        world,
        palette: PALETTES[0],
        frame: 40,
        width: 720,
        height: 260,
        compact: false,
        reducedMotion: false,
        nowMs: 3_600,
      }),
    ).not.toThrow();
    expect(() =>
      drawShowcaseEvent({
        ctx: makeCanvasContext(),
        run: makeShowcaseRun({
          kind: "stargazing_constellation",
          target: OBSERVATORY_TARGET,
          pose: "stargaze",
          speech:
            "Buddy reads the model stars and traces a careful constellation.",
        }),
        world,
        palette: PALETTES[0],
        frame: 40,
        width: 720,
        height: 260,
        compact: false,
        reducedMotion: false,
        nowMs: 3_600,
      }),
    ).not.toThrow();
  });

  it("draws reduced-motion compact showcase overlays", () => {
    expect(() =>
      drawShowcaseEvent({
        ctx: makeCanvasContext(),
        run: makeShowcaseRun({
          kind: "stargazing_constellation",
          target: OBSERVATORY_TARGET,
          pose: "stargaze",
        }),
        world: makeWorld(),
        palette: PALETTES[0],
        frame: 4,
        width: 360,
        height: 190,
        compact: true,
        reducedMotion: true,
        nowMs: 2_200,
      }),
    ).not.toThrow();
  });

  it("clamps all showcase draw alpha writes", () => {
    const world = makeWorld();
    const standardMemoryCtx = makeCanvasContext();
    const standardStargazingCtx = makeCanvasContext();
    const reducedCompactCtx = makeCanvasContext();

    drawShowcaseEvent({
      ctx: standardMemoryCtx,
      run: makeShowcaseRun(),
      world,
      palette: PALETTES[0],
      frame: 240,
      width: 720,
      height: 260,
      compact: false,
      reducedMotion: false,
      nowMs: 3_400,
    });
    drawShowcaseEvent({
      ctx: standardStargazingCtx,
      run: makeShowcaseRun({
        kind: "stargazing_constellation",
        target: OBSERVATORY_TARGET,
        pose: "stargaze",
      }),
      world,
      palette: PALETTES[0],
      frame: 240,
      width: 720,
      height: 260,
      compact: false,
      reducedMotion: false,
      nowMs: 2_800,
    });
    drawShowcaseEvent({
      ctx: reducedCompactCtx,
      run: makeShowcaseRun({
        kind: "stargazing_constellation",
        target: OBSERVATORY_TARGET,
        pose: "stargaze",
      }),
      world,
      palette: PALETTES[0],
      frame: 240,
      width: 360,
      height: 190,
      compact: true,
      reducedMotion: true,
      nowMs: 2_800,
    });

    expectAlphaWritesClamped(standardMemoryCtx);
    expectAlphaWritesClamped(standardStargazingCtx);
    expectAlphaWritesClamped(reducedCompactCtx);
  });

  it("does not write non-finite alpha or draw coordinates for edge draw inputs", () => {
    const invalidTimeCtx = makeCanvasContext();
    const invalidSeedCtx = makeCanvasContext();
    const invalidGeometryCtx = makeCanvasContext();

    drawShowcaseEvent({
      ctx: invalidTimeCtx,
      run: makeShowcaseRun(),
      world: makeWorld(),
      palette: PALETTES[0],
      frame: 240,
      width: 720,
      height: 260,
      compact: false,
      reducedMotion: false,
      nowMs: Number.NaN,
    });
    drawShowcaseEvent({
      ctx: invalidSeedCtx,
      run: makeShowcaseRun({
        kind: "stargazing_constellation",
        target: OBSERVATORY_TARGET,
        pose: "stargaze",
        seed: Number.POSITIVE_INFINITY,
      }),
      world: makeWorld(),
      palette: PALETTES[0],
      frame: 240,
      width: 720,
      height: 260,
      compact: false,
      reducedMotion: false,
      nowMs: 3_400,
    });
    drawShowcaseEvent({
      ctx: invalidGeometryCtx,
      run: makeShowcaseRun({
        kind: "stargazing_constellation",
        target: {
          ...OBSERVATORY_TARGET,
          x: Number.POSITIVE_INFINITY,
          y: Number.NaN,
        },
        pose: "stargaze",
        phaseStartedAtMs: Number.NEGATIVE_INFINITY,
      }),
      world: makeWorld(),
      palette: PALETTES[0],
      frame: Number.POSITIVE_INFINITY,
      width: Number.POSITIVE_INFINITY,
      height: Number.NaN,
      compact: false,
      reducedMotion: false,
      nowMs: Number.POSITIVE_INFINITY,
    });

    expectAlphaWritesFinite(invalidTimeCtx);
    expectAlphaWritesFinite(invalidSeedCtx);
    expectAlphaWritesFinite(invalidGeometryCtx);
    expectAlphaWritesClamped(invalidTimeCtx);
    expectAlphaWritesClamped(invalidSeedCtx);
    expectAlphaWritesClamped(invalidGeometryCtx);
    expectDrawOpsFinite(invalidTimeCtx);
    expectDrawOpsFinite(invalidSeedCtx);
    expectDrawOpsFinite(invalidGeometryCtx);
  });

  it("draws deterministic showcase output for the same seed and frame", () => {
    const firstCtx = makeCanvasContext();
    const secondCtx = makeCanvasContext();
    const run = makeShowcaseRun({
      kind: "stargazing_constellation",
      target: OBSERVATORY_TARGET,
      pose: "stargaze",
      seed: 98765,
    });
    const args = {
      run,
      world: makeWorld(),
      palette: PALETTES[0],
      frame: 180,
      width: 720,
      height: 260,
      compact: false,
      reducedMotion: false,
      nowMs: 3_600,
    };

    drawShowcaseEvent({ ctx: firstCtx, ...args });
    drawShowcaseEvent({ ctx: secondCtx, ...args });

    expect(secondCtx.drawOps).toEqual(firstCtx.drawOps);
  });

  it("chooses and creates memory firefly night for memory runtime signals", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({ signal_type: "knowledge_update" }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 10_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(chooseBuddyShowcase(args)?.kind).toBe("memory_firefly_night");
    const run = createBuddyShowcaseRun(args);

    expect(run).toMatchObject({
      kind: "memory_firefly_night",
      phase: "travel",
      target: {
        id: "memory",
        label: "Memory fireflies",
      },
      pose: "meditate",
      startedAtMs: 10_000,
      phaseStartedAtMs: 10_000,
    });
  });

  it("ignores dismissed memory runtime signals", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "knowledge_update",
        dismissed: true,
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 11_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(hasBuddyShowcaseRuntimeTrigger(args.nowPlaying)).toBe(false);
    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("ignores expired non-persistent runtime signals", () => {
    const nowMs = new Date("2024-01-01T00:02:00Z").getTime();
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "knowledge_update",
        created_at: "2024-01-01T00:00:00Z",
        ttl_ms: 30_000,
        persistent: false,
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(hasBuddyShowcaseRuntimeTrigger(args.nowPlaying, nowMs)).toBe(false);
    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("keeps persistent expired-looking runtime signals eligible", () => {
    const nowMs = new Date("2024-01-01T00:02:00Z").getTime();
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "knowledge_update",
        created_at: "2024-01-01T00:00:00Z",
        ttl_ms: 30_000,
        persistent: true,
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(hasBuddyShowcaseRuntimeTrigger(args.nowPlaying, nowMs)).toBe(true);
    expect(chooseBuddyShowcase(args)?.kind).toBe("memory_firefly_night");
  });

  it("treats invalid runtime timestamps as not expired", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "knowledge_update",
        created_at: "not-a-date",
        ttl_ms: 30_000,
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 14_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(hasBuddyShowcaseRuntimeTrigger(args.nowPlaying)).toBe(true);
    expect(chooseBuddyShowcase(args)?.kind).toBe("memory_firefly_night");
  });

  it("chooses and creates stargazing constellation for generation and provider signals", () => {
    const generatingArgs = {
      targets: [OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "streaming",
        title: "Streaming answer",
        status: "streaming",
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 20_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "evening" as const, weather: "busy" as const },
    };
    const providerArgs = {
      ...generatingArgs,
      nowPlaying: makeRuntimeEvent({
        signal_type: "error",
        title: "Provider quota warning",
        description: "The default model quota is low.",
        status: "failed",
      }),
    };

    expect(chooseBuddyShowcase(generatingArgs)?.kind).toBe(
      "stargazing_constellation",
    );
    expect(createBuddyShowcaseRun(generatingArgs)?.target.id).toBe("providers");
    expect(chooseBuddyShowcase(providerArgs)?.kind).toBe(
      "stargazing_constellation",
    );
  });

  it("uses provider fallback for failed mapped generation runtime signals", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "generating",
        title: "Default model missing",
        description: "The provider default chat model is missing.",
        source: "providers/default_models",
        status: "failed",
        priority: "normal",
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 20_200,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "evening" as const, weather: "busy" as const },
    };

    expect(hasBuddyShowcaseRuntimeTrigger(args.nowPlaying)).toBe(true);
    expect(chooseBuddyShowcase(args)?.kind).toBe("stargazing_constellation");
    expect(createBuddyShowcaseRun(args)?.target.id).toBe("providers");
  });

  it("uses provider fallback for failed mapped memory runtime signals", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "memory_extract",
        title: "Provider model not found",
        description: "The default model reference is missing.",
        source: "providers/default_models",
        status: "failed",
        priority: "normal",
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 20_300,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(hasBuddyShowcaseRuntimeTrigger(args.nowPlaying)).toBe(true);
    expect(chooseBuddyShowcase(args)?.kind).toBe("stargazing_constellation");
    expect(createBuddyShowcaseRun(args)?.target.id).toBe("providers");
  });

  it("ignores dismissed provider and generation runtime signals", () => {
    const baseArgs = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 20_500,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "evening" as const, weather: "busy" as const },
    };
    const streamingEvent = makeRuntimeEvent({
      signal_type: "streaming",
      title: "Streaming answer",
      status: "streaming",
      dismissed: true,
    });
    const providerEvent = makeRuntimeEvent({
      signal_type: "error",
      title: "Provider quota warning",
      description: "The default model quota is low.",
      status: "failed",
      dismissed: true,
    });

    expect(hasBuddyShowcaseRuntimeTrigger(streamingEvent)).toBe(false);
    expect(hasBuddyShowcaseRuntimeTrigger(providerEvent)).toBe(false);
    expect(
      chooseBuddyShowcase({ ...baseArgs, nowPlaying: streamingEvent }),
    ).toBeNull();
    expect(
      chooseBuddyShowcase({ ...baseArgs, nowPlaying: providerEvent }),
    ).toBeNull();
  });

  it("ignores completed active-work runtime signals", () => {
    const args = {
      targets: [OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "streaming",
        title: "Streaming finished",
        status: "completed",
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 21_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "evening" as const, weather: "busy" as const },
    };

    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("ignores failed memory runtime signals", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "memory_extract",
        title: "Memory extraction failed",
        status: "failed",
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 22_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("only treats meaningful memory statuses as runtime triggers", () => {
    const baseArgs = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 23_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(
      chooseBuddyShowcase({
        ...baseArgs,
        nowPlaying: makeRuntimeEvent({
          signal_type: "memory_extract",
          status: "info",
        }),
      }),
    ).toBeNull();
    expect(
      chooseBuddyShowcase({
        ...baseArgs,
        nowPlaying: makeRuntimeEvent({
          signal_type: "knowledge_update",
          status: "started",
        }),
      }),
    ).toBeNull();
    expect(
      chooseBuddyShowcase({
        ...baseArgs,
        nowPlaying: makeRuntimeEvent({
          signal_type: "knowledge_update",
          status: "progress",
        }),
      })?.kind,
    ).toBe("memory_firefly_night");
  });

  it("initial idle grace blocks idle starts but not explicit runtime triggers", () => {
    const idleArgs = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: null,
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 2_000,
      idleGraceUntilMs: BUDDY_SHOWCASE_INITIAL_GRACE_MS,
      lastShowcaseKind: null,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };
    const runtimeArgs = {
      ...idleArgs,
      nowPlaying: makeRuntimeEvent({ signal_type: "memory_extract" }),
      strongRuntimeTrigger: true,
    };

    expect(chooseBuddyShowcase(idleArgs)).toBeNull();
    expect(chooseBuddyShowcase(runtimeArgs)?.kind).toBe("memory_firefly_night");
  });

  it("applies separate runtime and idle showcase cooldowns", () => {
    const baseArgs = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 50_000,
      idleGraceUntilMs: BUDDY_SHOWCASE_INITIAL_GRACE_MS,
      lastShowcaseKind: null,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };
    const runtimeArgs = {
      ...baseArgs,
      nowPlaying: makeRuntimeEvent({
        id: "runtime-new",
        signal_type: "memory_extract",
      }),
      idleCooldownUntilMs: baseArgs.nowMs + BUDDY_SHOWCASE_IDLE_COOLDOWN_MS,
      runtimeCooldownUntilMs: baseArgs.nowMs - 1,
      runtimeShowcaseEventIds: ["runtime-old"],
      strongRuntimeTrigger: true,
    };

    expect(chooseBuddyShowcase(runtimeArgs)?.kind).toBe("memory_firefly_night");
    expect(
      chooseBuddyShowcase({
        ...runtimeArgs,
        runtimeCooldownUntilMs:
          baseArgs.nowMs + BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS,
      }),
    ).toBeNull();
    expect(
      chooseBuddyShowcase({
        ...baseArgs,
        nowPlaying: null,
        idleCooldownUntilMs: baseArgs.nowMs + BUDDY_SHOWCASE_IDLE_COOLDOWN_MS,
      }),
    ).toBeNull();
  });

  it("suppresses repeated showcases for the same runtime event id", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        id: "runtime-same",
        signal_type: "memory_extract",
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 44_000,
      runtimeCooldownUntilMs: 44_000 - BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS,
      runtimeShowcaseEventIds: ["runtime-other", "runtime-same"],
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("allows a new runtime event between remembered runtime event ids", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        id: "runtime-new",
        signal_type: "memory_extract",
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 45_000,
      runtimeCooldownUntilMs: 45_000 - BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS,
      runtimeShowcaseEventIds: ["runtime-a", "runtime-b"],
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(chooseBuddyShowcase(args)?.kind).toBe("memory_firefly_night");
  });

  it("returns null for unmapped strong runtime signals", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "chat_started",
        title: "Chat started",
        status: "started",
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 25_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "aurora" as const },
    };

    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("does not treat generic model mentions as provider runtime triggers", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "error",
        title: "Model answered slowly",
        description: "A generic model note without provider details.",
        status: "info",
        priority: "normal",
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 26_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "evening" as const, weather: "clear" as const },
    };

    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("ignores benign high-priority provider notifications", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "provider_sync",
        title: "Provider settings saved",
        description: "Provider configuration completed successfully.",
        status: "completed",
        priority: "high",
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 27_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "evening" as const, weather: "clear" as const },
    };

    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("ignores benign successful default model and provider sync notifications", () => {
    const baseArgs = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 27_500,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "evening" as const, weather: "clear" as const },
    };
    const defaultModelsEvent = makeRuntimeEvent({
      signal_type: "default_models_sync",
      title: "Default models synced",
      description: "Provider defaults refreshed successfully with no warnings.",
      source: "providers/default_models",
      status: "completed",
      priority: "critical",
    });
    const providerSyncEvent = makeRuntimeEvent({
      signal_type: "provider_sync",
      title: "Provider catalog refreshed",
      description: "All provider sources synced successfully.",
      source: "providers",
      status: "completed",
      priority: "normal",
    });

    expect(hasBuddyShowcaseRuntimeTrigger(defaultModelsEvent)).toBe(false);
    expect(hasBuddyShowcaseRuntimeTrigger(providerSyncEvent)).toBe(false);
    expect(
      chooseBuddyShowcase({ ...baseArgs, nowPlaying: defaultModelsEvent }),
    ).toBeNull();
    expect(
      chooseBuddyShowcase({ ...baseArgs, nowPlaying: providerSyncEvent }),
    ).toBeNull();
  });

  it.each([
    ["no errors", "Provider defaults refreshed with no errors."],
    ["no problems", "Provider scan finished with no problems."],
    ["no missing references", "Default models have no missing references."],
    ["no warnings", "Provider sources synced with no warnings."],
    [
      "no errors or warnings",
      "Default model sync found no errors or warnings.",
    ],
  ] as const)(
    "ignores benign provider notifications with %s",
    (_label, description) => {
      const args = {
        targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
        nowPlaying: makeRuntimeEvent({
          signal_type: "provider_sync",
          title: "Provider sync complete",
          description,
          source: "providers/default_models",
          status: "completed",
          priority: "critical",
        }),
        activeSpeechVisible: false,
        pet: makePet(),
        nowMs: 27_600,
        lastShowcaseKind: null,
        strongRuntimeTrigger: true,
        pulse: makePulse(),
        world: { phase: "evening" as const, weather: "clear" as const },
      };

      expect(hasBuddyShowcaseRuntimeTrigger(args.nowPlaying)).toBe(false);
      expect(chooseBuddyShowcase(args)).toBeNull();
      expect(createBuddyShowcaseRun(args)).toBeNull();
    },
  );

  it.each([
    [
      "model not found",
      "Provider model not found",
      "The configured model failed lookup.",
    ],
    [
      "quota warning",
      "Provider quota warning",
      "The default model quota is low.",
    ],
    ["broken ref", "Provider broken ref", "A model reference is invalid."],
    [
      "missing default model",
      "Missing default model",
      "The provider default is unset.",
    ],
  ] as const)(
    "keeps real provider problem notifications eligible for %s",
    (_label, title, description) => {
      const args = {
        targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
        nowPlaying: makeRuntimeEvent({
          signal_type: "provider_sync",
          title,
          description,
          source: "providers/default_models",
          status: "completed",
          priority: "normal",
        }),
        activeSpeechVisible: false,
        pet: makePet(),
        nowMs: 27_700,
        lastShowcaseKind: null,
        strongRuntimeTrigger: true,
        pulse: makePulse(),
        world: { phase: "evening" as const, weather: "clear" as const },
      };

      expect(hasBuddyShowcaseRuntimeTrigger(args.nowPlaying)).toBe(true);
      expect(chooseBuddyShowcase(args)?.kind).toBe("stargazing_constellation");
    },
  );

  it("keeps real failed provider problem notifications eligible", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        signal_type: "provider_sync",
        title: "Provider model not found",
        description: "The default chat model reference is missing.",
        source: "providers/default_models",
        status: "failed",
        priority: "normal",
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 27_800,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "evening" as const, weather: "clear" as const },
    };

    expect(hasBuddyShowcaseRuntimeTrigger(args.nowPlaying)).toBe(true);
    expect(chooseBuddyShowcase(args)?.kind).toBe("stargazing_constellation");
    expect(createBuddyShowcaseRun(args)?.target.id).toBe("providers");
  });

  it("provider pulse issues prefer and create stargazing constellation", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: null,
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 28_000,
      lastShowcaseKind: null,
      pulse: makePulse({
        providers: { defaults_ok: false, broken_refs: 1, quota_warnings: 1 },
      }),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(chooseBuddyShowcase(args)?.kind).toBe("stargazing_constellation");
    expect(createBuddyShowcaseRun(args)?.target.id).toBe("providers");
  });

  it("provider pulse issues respect idle repeat soft-ban", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: null,
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 156_000,
      lastShowcaseKind: "stargazing_constellation" as const,
      pulse: makePulse({
        providers: { defaults_ok: false, broken_refs: 99, quota_warnings: 99 },
        memory: { total: 8, orphan: 1, stale_conflicts: 0 },
      }),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(chooseBuddyShowcase(args)?.kind).toBe("memory_firefly_night");
  });

  it("memory pulse and night context prefer memory firefly night", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: null,
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 29_000,
      lastShowcaseKind: null,
      pulse: makePulse({
        memory: { total: 50, orphan: 3, stale_conflicts: 1 },
      }),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(chooseBuddyShowcase(args)?.kind).toBe("memory_firefly_night");
    expect(createBuddyShowcaseRun(args)?.target.id).toBe("memory");
  });

  it("idle weighted selection is deterministic without always following highest score", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: null,
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 156_000,
      lastShowcaseKind: null,
      pulse: makePulse({
        memory: { total: 50, orphan: 3, stale_conflicts: 1 },
      }),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    const firstChoice = chooseBuddyShowcase(args)?.kind;
    const secondChoice = chooseBuddyShowcase(args)?.kind;

    expect(secondChoice).toBe(firstChoice);
    expect(firstChoice).toBe("stargazing_constellation");
  });

  it("idle weighted selection is stable within the same time bucket", () => {
    const args = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: null,
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: BUDDY_SHOWCASE_IDLE_COOLDOWN_MS * 4 + 10,
      lastShowcaseKind: "memory_firefly_night" as const,
      pulse: makePulse({
        memory: { total: 18, orphan: 2, stale_conflicts: 1 },
      }),
      world: { phase: "evening" as const, weather: "aurora" as const },
    };

    const firstChoice = chooseBuddyShowcase(args)?.kind;
    const secondChoice = chooseBuddyShowcase({
      ...args,
      nowMs: args.nowMs + BUDDY_SHOWCASE_IDLE_COOLDOWN_MS - 20,
    })?.kind;

    expect(secondChoice).toBe(firstChoice);
  });

  it("active speech suppresses chooser and run creation", () => {
    const args = {
      targets: [MEMORY_TARGET],
      nowPlaying: makeRuntimeEvent(),
      activeSpeechVisible: true,
      pet: makePet(),
      nowMs: 30_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("local visible speech suppresses chooser and run creation", () => {
    const args = {
      targets: [MEMORY_TARGET],
      nowPlaying: makeRuntimeEvent(),
      activeSpeechVisible: true,
      pet: makePet(),
      nowMs: 32_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "evening" as const, weather: "clear" as const },
    };

    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("sleep and cooldown suppress chooser and run creation", () => {
    const sleepingArgs = {
      targets: [MEMORY_TARGET],
      nowPlaying: makeRuntimeEvent(),
      activeSpeechVisible: false,
      pet: makePet(true),
      nowMs: 35_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };
    const cooldownArgs = {
      ...sleepingArgs,
      pet: makePet(false),
      runtimeCooldownUntilMs: 40_000,
    };

    expect(chooseBuddyShowcase(sleepingArgs)).toBeNull();
    expect(createBuddyShowcaseRun(sleepingArgs)).toBeNull();
    expect(chooseBuddyShowcase(cooldownArgs)).toBeNull();
    expect(createBuddyShowcaseRun(cooldownArgs)).toBeNull();
  });

  it("returns null when the required target is missing", () => {
    const args = {
      targets: [OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({ signal_type: "memory_extract" }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 40_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("does not match undefined sprite fallback when target id is missing", () => {
    const args = {
      targets: [
        {
          id: "future-target",
          x: 48,
          y: 58,
          label: "Future target",
        },
      ],
      nowPlaying: makeRuntimeEvent({ signal_type: "memory_extract" }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 41_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
  });

  it("avoids immediate idle repeat unless a strong runtime trigger exists", () => {
    const idleArgs = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: null,
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 350_000,
      lastShowcaseKind: "memory_firefly_night" as const,
      pulse: makePulse(),
      world: { phase: "day" as const, weather: "clear" as const },
    };
    const strongArgs = {
      ...idleArgs,
      targets: [MEMORY_TARGET],
      nowPlaying: makeRuntimeEvent({ signal_type: "memory_extract" }),
      strongRuntimeTrigger: true,
    };

    expect(chooseBuddyShowcase(idleArgs)?.kind).toBe(
      "stargazing_constellation",
    );
    expect(chooseBuddyShowcase(strongArgs)?.kind).toBe("memory_firefly_night");
  });

  it("suppresses runtime ids with bounded recent runtime memory", () => {
    const recentIds = Array.from(
      { length: 20 },
      (_, index) => `runtime-${index}`,
    );
    const currentArgs = {
      targets: [MEMORY_TARGET, OBSERVATORY_TARGET],
      nowPlaying: makeRuntimeEvent({
        id: "runtime-12",
        signal_type: "knowledge_update",
      }),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 48_000,
      runtimeCooldownUntilMs: 48_000 - BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS,
      runtimeShowcaseEventIds: recentIds.slice(0, 16),
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };
    const expiredArgs = {
      ...currentArgs,
      nowPlaying: makeRuntimeEvent({
        id: "runtime-19",
        signal_type: "knowledge_update",
      }),
    };

    expect(chooseBuddyShowcase(currentArgs)).toBeNull();
    expect(chooseBuddyShowcase(expiredArgs)?.kind).toBe("memory_firefly_night");
  });

  it("phase advancement reaches null after cooldown", () => {
    const run = createBuddyShowcaseRun({
      targets: [MEMORY_TARGET],
      nowPlaying: makeRuntimeEvent(),
      activeSpeechVisible: false,
      pet: makePet(),
      nowMs: 60_000,
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    });
    expect(run).not.toBeNull();

    let current = run;
    let nowMs = 60_000;
    for (const phase of [
      "travel",
      "anticipate",
      "showcase",
      "react",
      "cooldown",
    ] as const) {
      expect(current?.phase).toBe(phase);
      nowMs += BUDDY_SHOWCASE_PHASE_DURATIONS_MS[phase];
      current = current
        ? advanceBuddyShowcasePhase({ run: current, nowMs })
        : null;
    }

    expect(current).toBeNull();
  });
});
