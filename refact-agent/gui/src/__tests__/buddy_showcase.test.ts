import { describe, expect, it, vi } from "vitest";
import {
  advanceBuddyShowcasePhase,
  BUDDY_SHOWCASE_INITIAL_GRACE_MS,
  BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS,
  BUDDY_SHOWCASE_PHASE_DURATIONS_MS,
  chooseBuddyShowcase,
  createBuddyShowcaseRun,
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
  | "setTransform"
  | "stroke"
> &
  Partial<CanvasRenderingContext2D>;

function makeCanvasContext(): CanvasRenderingContext2D {
  const gradient = { addColorStop: vi.fn() } as unknown as CanvasGradient;
  const ctx: MockCanvasContext = {
    arc: vi.fn(),
    beginPath: vi.fn(),
    bezierCurveTo: vi.fn(),
    clearRect: vi.fn(),
    closePath: vi.fn(),
    createLinearGradient: vi.fn(() => gradient),
    ellipse: vi.fn(),
    fill: vi.fn(),
    fillRect: vi.fn(),
    fillText: vi.fn(),
    lineTo: vi.fn(),
    moveTo: vi.fn(),
    restore: vi.fn(),
    save: vi.fn(),
    setTransform: vi.fn(),
    stroke: vi.fn(),
    fillStyle: "#000000",
    font: "10px monospace",
    globalAlpha: 1,
    imageSmoothingEnabled: false,
    lineCap: "round" as CanvasLineCap,
    lineWidth: 1,
    strokeStyle: "#000000",
    textAlign: "center" as CanvasTextAlign,
    textBaseline: "middle" as CanvasTextBaseline,
  };
  return ctx as CanvasRenderingContext2D;
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
      cooldownUntilMs: 44_000 - BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS,
      lastRuntimeShowcaseEventId: "runtime-same",
      lastShowcaseKind: null,
      strongRuntimeTrigger: true,
      pulse: makePulse(),
      world: { phase: "night" as const, weather: "rain" as const },
    };

    expect(chooseBuddyShowcase(args)).toBeNull();
    expect(createBuddyShowcaseRun(args)).toBeNull();
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
      cooldownUntilMs: 40_000,
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
