import { describe, expect, it } from "vitest";
import { buildBuddyWorldState } from "../features/Buddy/buddyWorldModel";
import type {
  BuddyPetState,
  BuddyPulse,
  BuddyRuntimeEvent,
  BuddySemanticState,
} from "../features/Buddy/types";

function makePulse(overrides?: Partial<BuddyPulse>): BuddyPulse {
  return {
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
    ...overrides,
  };
}

function makePet(args?: {
  condition?: Partial<BuddyPetState["condition"]>;
  needs?: Partial<BuddyPetState["needs"]>;
}): BuddyPetState {
  return {
    needs: {
      hunger: 80,
      energy: 80,
      hygiene: 80,
      boredom: 10,
      affection: 35,
      ...args?.needs,
    },
    condition: {
      sleeping: false,
      hungry: false,
      sleepy: false,
      dirty: false,
      bored: false,
      lonely: false,
      ...args?.condition,
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
    signal_type: "streaming",
    title: "Streaming answer",
    source: "test",
    status: "streaming",
    priority: "normal",
    created_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

function makeSemanticState(
  overrides?: Partial<BuddySemanticState>,
): BuddySemanticState {
  return {
    name: "Buddy",
    paletteIndex: 0,
    born: 0,
    mood: {
      happiness: 80,
      energy: 80,
      curiosity: 70,
      anxiety: 0,
      boredom: 10,
      affection: 80,
    },
    personality: {
      playfulness: 70,
      confidence: 60,
      clinginess: 70,
      resilience: 60,
      chaos: 30,
      sociability: 70,
      curiosity: 70,
    },
    progress: { xp: 0, stage: 2 },
    activity: {
      mood: "idle",
      animationType: "idle",
      lastSignalTime: 0,
      lastSignalType: null,
    },
    skills: [],
    log: [],
    ...overrides,
  };
}

function buildWorld(args?: {
  hour?: number;
  pulse?: BuddyPulse | null;
  pet?: BuddyPetState;
  nowPlaying?: BuddyRuntimeEvent | null;
  semanticState?: BuddySemanticState;
}) {
  return buildBuddyWorldState({
    now: new Date(2024, 0, 1, args?.hour ?? 14, 0, 0),
    pulse: args && "pulse" in args ? args.pulse : makePulse(),
    pet: args?.pet ?? makePet(),
    nowPlaying: args?.nowPlaying ?? null,
    activeQuest: null,
    semanticState: args?.semanticState,
  });
}

function expectWorldNumbersSafe(world: ReturnType<typeof buildWorld>): void {
  const worldValues = [
    world.celestialX,
    world.celestialY,
    world.weatherX,
    world.weatherY,
    world.atmosphere.intensity,
  ];
  expect(worldValues.every((value) => Number.isFinite(value))).toBe(true);
  expect(Number.isFinite(world.atmosphere.intensity)).toBe(true);
  expect(world.atmosphere.intensity).toBeGreaterThanOrEqual(0);
  expect(world.atmosphere.intensity).toBeLessThanOrEqual(1);
  for (const item of world.objects) {
    const values = [
      item.x,
      item.y,
      item.size,
      item.intensity,
      item.interactionX,
      item.interactionY,
      item.depthScale,
    ];
    expect(values.every((value) => Number.isFinite(value))).toBe(true);
    expect(item.intensity).toBeGreaterThanOrEqual(0);
    expect(item.intensity).toBeLessThanOrEqual(1);
    expect(item.interactionX).toBeGreaterThanOrEqual(0);
    expect(item.interactionX).toBeLessThanOrEqual(100);
    expect(item.interactionY).toBeGreaterThanOrEqual(58);
    expect(item.interactionY).toBeLessThanOrEqual(84);
    expect(item.depthScale).toBeGreaterThan(0);
  }
}

function getProviderObject(world: ReturnType<typeof buildWorld>) {
  const providers = world.objects.find((item) => item.id === "providers");
  if (!providers) throw new Error("Provider object missing");
  return providers;
}

describe("buddy world semantic model", () => {
  it("maps morning, day, evening, and night to distinct palettes and layers", () => {
    const morning = buildWorld({ hour: 8 });
    const day = buildWorld({ hour: 13 });
    const evening = buildWorld({ hour: 18 });
    const night = buildWorld({ hour: 23 });

    expect(morning.atmosphere.paletteHint).toBe("dawn");
    expect(morning.atmosphere.layers).toContain("sun_motes");
    expect(day.atmosphere.paletteHint).toBe("day");
    expect(day.atmosphere.layers).toContain("sun_motes");
    expect(evening.atmosphere.paletteHint).toBe("dusk");
    expect(evening.atmosphere.layers).toEqual(
      expect.arrayContaining(["moths", "cozy_home_glow"]),
    );
    expect(night.atmosphere.paletteHint).toBe("night");
    expect(night.atmosphere.layers).toEqual(
      expect.arrayContaining(["stars", "fireflies"]),
    );
  });

  it("turns a sleeping pet into dream mist and dream weather", () => {
    const world = buildWorld({
      pet: makePet({ condition: { sleeping: true } }),
    });

    expect(world.weather).toBe("dream");
    expect(world.atmosphere.primaryWeather).toBe("dream");
    expect(world.atmosphere.paletteHint).toBe("dream");
    expect(world.atmosphere.mood).toBe("sleepy");
    expect(world.atmosphere.layers).toContain("dream_mist");
  });

  it("keeps serious provider storms visible while Buddy sleeps", () => {
    const world = buildWorld({
      pet: makePet({ condition: { sleeping: true } }),
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 1, quota_warnings: 0 },
      }),
    });

    expect(world.weather).toBe("storm");
    expect(world.atmosphere.primaryWeather).toBe("storm");
    expect(world.atmosphere.serious).toBe(true);
    expect(world.atmosphere.layers).toEqual(
      expect.arrayContaining(["provider_storm", "dream_mist"]),
    );
  });

  it("adds subtle care layers for hunger, boredom, and affection", () => {
    const hungry = buildWorld({
      pet: makePet({ condition: { hungry: true } }),
    });
    const bored = buildWorld({
      pet: makePet({ condition: { bored: true } }),
    });
    const affectionate = buildWorld({
      pet: makePet({ needs: { affection: 90 } }),
    });
    const recentAffection = buildWorld({
      semanticState: makeSemanticState({
        activity: {
          mood: "happy",
          animationType: "perk",
          lastSignalTime: new Date(2024, 0, 1, 13, 58, 0).getTime(),
          lastSignalType: "care_pet",
        },
      }),
    });

    expect(hungry.atmosphere.mood).toBe("hungry");
    expect(hungry.atmosphere.layers).toContain("empty_food_nook");
    expect(bored.atmosphere.mood).toBe("bored");
    expect(bored.atmosphere.layers).toContain("toy_glow");
    expect(affectionate.atmosphere.mood).toBe("affectionate");
    expect(affectionate.atmosphere.layers).toContain("cozy_home_glow");
    expect(recentAffection.atmosphere.mood).toBe("affectionate");
    expect(recentAffection.atmosphere.layers).toContain("cozy_home_glow");
  });

  it("treats slightly future affection signals as recent", () => {
    const world = buildWorld({
      pet: makePet({ needs: { affection: 35 } }),
      semanticState: makeSemanticState({
        activity: {
          mood: "happy",
          animationType: "perk",
          lastSignalTime: new Date(2024, 0, 1, 14, 0, 3).getTime(),
          lastSignalType: "care_pet",
        },
      }),
    });

    expect(world.atmosphere.mood).toBe("affectionate");
    expect(world.atmosphere.layers).toContain("cozy_home_glow");
  });

  it("adds workshop runes and active observatory state for visible active runtime", () => {
    const world = buildWorld({ nowPlaying: makeRuntimeEvent() });
    const providers = getProviderObject(world);

    expect(world.weather).toBe("busy");
    expect(world.atmosphere.mood).toBe("busy");
    expect(world.atmosphere.layers).toContain("workshop_runes");
    expect(providers).toMatchObject({
      state: "active",
      animation: "stream",
    });
  });

  it("keeps generic active runtime in workshop work without activating observatory", () => {
    const world = buildWorld({
      nowPlaying: makeRuntimeEvent({
        signal_type: "tool_used",
        status: "progress",
        title: "Running browser checks",
        description: "Clicking a page button",
        source: "browser",
      }),
    });
    const providers = getProviderObject(world);

    expect(world.weather).toBe("busy");
    expect(world.atmosphere.layers).toContain("workshop_runes");
    expect(providers).toMatchObject({
      state: "calm",
      animation: "sparkle",
    });
  });

  it("ignores dismissed and expired active runtime events for busy atmosphere", () => {
    const dismissed = buildWorld({
      pulse: makePulse({
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
      }),
      nowPlaying: makeRuntimeEvent({ dismissed: true }),
    });
    const expired = buildBuddyWorldState({
      now: new Date("2024-01-01T14:00:00Z"),
      pulse: makePulse({
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
      }),
      pet: makePet(),
      nowPlaying: makeRuntimeEvent({
        created_at: "2024-01-01T13:58:00Z",
        ttl_ms: 30_000,
        persistent: false,
      }),
      activeQuest: null,
    });

    expect(dismissed.weather).not.toBe("busy");
    expect(dismissed.atmosphere.layers).not.toContain("workshop_runes");
    expect(expired.weather).not.toBe("busy");
    expect(expired.atmosphere.layers).not.toContain("workshop_runes");
  });

  it("maps provider warnings to attention and flicker without storms", () => {
    const world = buildWorld({
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 1 },
      }),
    });
    const providers = getProviderObject(world);

    expect(providers).toMatchObject({
      state: "attention",
      animation: "flicker",
    });
    expect(world.weather).not.toBe("storm");
    expect(world.atmosphere.serious).toBe(false);
    expect(world.atmosphere.layers).toContain("provider_flicker");
    expect(world.atmosphere.layers).not.toContain("provider_storm");
  });

  it("keeps defaults warnings flickering without provider storms", () => {
    const world = buildWorld({
      pulse: makePulse({
        providers: { defaults_ok: false, broken_refs: 0, quota_warnings: 0 },
      }),
    });
    const providers = getProviderObject(world);

    expect(providers).toMatchObject({
      state: "attention",
      animation: "flicker",
    });
    expect(world.weather).not.toBe("storm");
    expect(world.atmosphere.serious).toBe(false);
    expect(world.atmosphere.layers).toContain("provider_flicker");
    expect(world.atmosphere.layers).not.toContain("provider_storm");
  });

  it("keeps high generic diagnostics out of provider storm semantics", () => {
    const world = buildWorld({
      pulse: makePulse({
        diagnostics: {
          last_hour: 8,
          top_error_types: ["tool_failed", "browser_failure"],
        },
      }),
    });
    const providers = getProviderObject(world);

    expect(world.weather).not.toBe("storm");
    expect(world.atmosphere.serious).toBe(false);
    expect(world.atmosphere.paletteHint).not.toBe("storm");
    expect(world.atmosphere.layers).toContain("workshop_runes");
    expect(world.atmosphere.layers).not.toContain("provider_storm");
    expect(providers).not.toMatchObject({
      state: "critical",
      animation: "storm",
    });
  });

  it("detects high provider diagnostics even when defaults are healthy", () => {
    const world = buildWorld({
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 0 },
        diagnostics: {
          last_hour: 8,
          top_error_types: ["model_not_found"],
        },
      }),
    });
    const providers = getProviderObject(world);

    expect(world.weather).toBe("storm");
    expect(world.atmosphere.serious).toBe(true);
    expect(world.atmosphere.paletteHint).toBe("storm");
    expect(world.atmosphere.layers).toContain("provider_storm");
    expect(providers).toMatchObject({
      state: "critical",
      animation: "storm",
    });
  });

  it("keeps high generic failed runtime events out of provider storms", () => {
    const world = buildWorld({
      pulse: makePulse({
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
      }),
      nowPlaying: makeRuntimeEvent({
        signal_type: "tool_failed",
        status: "failed",
        priority: "high",
        title: "Browser action failed",
        description: "The page button was not found",
        source: "browser",
      }),
    });
    const providers = getProviderObject(world);

    expect(world.weather).not.toBe("storm");
    expect(world.atmosphere.serious).toBe(false);
    expect(world.atmosphere.layers).not.toContain("provider_storm");
    expect(providers).not.toMatchObject({
      state: "critical",
      animation: "storm",
    });
  });

  it("keeps failed git broken refs events out of provider storms", () => {
    const world = buildWorld({
      pulse: makePulse({
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
      }),
      nowPlaying: makeRuntimeEvent({
        signal_type: "tool_failed",
        status: "failed",
        priority: "high",
        title: "Git refs failed",
        description: "The local repository has broken refs.",
        source: "git",
      }),
    });
    const providers = getProviderObject(world);

    expect(world.weather).not.toBe("storm");
    expect(world.atmosphere.serious).toBe(false);
    expect(world.atmosphere.layers).not.toContain("provider_storm");
    expect(providers).not.toMatchObject({
      state: "critical",
      animation: "storm",
    });
  });

  it("keeps explicit broken model references stormy", () => {
    const world = buildWorld({
      pulse: makePulse({
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
      }),
      nowPlaying: makeRuntimeEvent({
        signal_type: "tool_failed",
        status: "failed",
        priority: "high",
        title: "Broken model reference",
        description: "The configured chat model reference is missing.",
        source: "git",
      }),
    });
    const providers = getProviderObject(world);

    expect(world.weather).toBe("storm");
    expect(world.atmosphere.serious).toBe(true);
    expect(world.atmosphere.layers).toContain("provider_storm");
    expect(providers).toMatchObject({
      state: "critical",
      animation: "storm",
      tone: "danger",
    });
  });

  it("keeps failed provider runtime events critical and stormy", () => {
    const world = buildWorld({
      pulse: makePulse({
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
      }),
      nowPlaying: makeRuntimeEvent({
        signal_type: "tool_failed",
        status: "failed",
        priority: "high",
        title: "Default model failed",
        description: "model_not_found for the configured chat model",
        source: "provider",
      }),
    });
    const providers = getProviderObject(world);

    expect(world.weather).toBe("storm");
    expect(world.atmosphere.serious).toBe(true);
    expect(world.atmosphere.paletteHint).toBe("storm");
    expect(world.atmosphere.layers).toContain("provider_storm");
    expect(providers).toMatchObject({
      state: "critical",
      animation: "storm",
      tone: "danger",
    });
  });

  it("marks critical provider objects with danger tone", () => {
    const world = buildWorld({
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 1, quota_warnings: 0 },
      }),
    });

    expect(getProviderObject(world)).toMatchObject({
      state: "critical",
      tone: "danger",
    });
  });

  it("does not treat unrelated GitHub API key failures as provider storms", () => {
    const world = buildWorld({
      pulse: makePulse({
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
      }),
      nowPlaying: makeRuntimeEvent({
        signal_type: "tool_failed",
        status: "failed",
        priority: "high",
        title: "GitHub API key missing",
        description: "The GitHub integration API key was not configured.",
        source: "github",
      }),
    });
    const providers = getProviderObject(world);

    expect(world.weather).not.toBe("storm");
    expect(world.atmosphere.serious).toBe(false);
    expect(world.atmosphere.layers).not.toContain("provider_storm");
    expect(providers).not.toMatchObject({
      state: "critical",
      animation: "storm",
    });
  });

  it("does not treat unrelated browser rate limits as provider storms", () => {
    const world = buildWorld({
      pulse: makePulse({
        git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
      }),
      nowPlaying: makeRuntimeEvent({
        signal_type: "tool_failed",
        status: "failed",
        priority: "high",
        title: "Browser rate limit reached",
        description: "The browser automation API rate limit was reached.",
        source: "browser",
      }),
    });
    const providers = getProviderObject(world);

    expect(world.weather).not.toBe("storm");
    expect(world.atmosphere.serious).toBe(false);
    expect(world.atmosphere.layers).not.toContain("provider_storm");
    expect(providers).not.toMatchObject({
      state: "critical",
      animation: "storm",
    });
  });

  it.each([
    ["api key", "Default model API key rejected"],
    ["rate limit", "Default model rate limit reached"],
  ] as const)(
    "keeps provider/default-model %s failures stormy",
    (_label, title) => {
      const world = buildWorld({
        pulse: makePulse({
          git: { uncommitted_files: 0, diff_lines_4h: 0, branches: 3 },
        }),
        nowPlaying: makeRuntimeEvent({
          signal_type: "tool_failed",
          status: "failed",
          priority: "high",
          title,
          description: "The configured provider default model could not run.",
          source: "providers/default_models",
        }),
      });
      const providers = getProviderObject(world);

      expect(world.weather).toBe("storm");
      expect(world.atmosphere.serious).toBe(true);
      expect(world.atmosphere.paletteHint).toBe("storm");
      expect(world.atmosphere.layers).toContain("provider_storm");
      expect(providers).toMatchObject({
        state: "critical",
        animation: "storm",
      });
    },
  );

  it("reserves storm semantics for serious provider issues", () => {
    const world = buildWorld({
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 1, quota_warnings: 0 },
      }),
    });
    const providers = getProviderObject(world);

    expect(world.weather).toBe("storm");
    expect(world.atmosphere.serious).toBe(true);
    expect(world.atmosphere.paletteHint).toBe("storm");
    expect(world.atmosphere.layers).toContain("provider_storm");
    expect(providers).toMatchObject({
      state: "critical",
      animation: "storm",
    });
  });

  it("makes memory pressure magical without provider storms", () => {
    const world = buildWorld({
      pulse: makePulse({
        memory: { total: 40, orphan: 3, stale_conflicts: 1 },
      }),
    });
    const memory = world.objects.find((item) => item.id === "memory");

    expect(world.atmosphere.layers).toContain("memory_orbs");
    expect(world.atmosphere.layers).not.toContain("provider_storm");
    expect(world.atmosphere.serious).toBe(false);
    expect(memory?.state).toBe("attention");
    expect(memory?.animation).toBe("orbit");
  });

  it("activates the memory object for visible memory runtime", () => {
    const world = buildWorld({
      nowPlaying: makeRuntimeEvent({
        signal_type: "memory_extract",
        status: "progress",
        title: "Gathering memory sparks",
      }),
    });
    const memory = world.objects.find((item) => item.id === "memory");

    expect(world.atmosphere.layers).toEqual(
      expect.arrayContaining(["workshop_runes", "memory_orbs"]),
    );
    expect(memory).toMatchObject({
      state: "active",
      animation: "stream",
    });
  });

  it("keeps all enriched object numbers and atmosphere intensity finite and clamped", () => {
    const world = buildWorld({
      pulse: makePulse({
        tasks: { total: 99, stuck: 99, abandoned: 99, by_status: {} },
        memory: { total: 99, orphan: 99, stale_conflicts: 99 },
        providers: { defaults_ok: false, broken_refs: 99, quota_warnings: 99 },
        mcp: { total: 99, failing: 99, auth_expiring: 99 },
        git: { uncommitted_files: 999, diff_lines_4h: 999, branches: 99 },
        diagnostics: { last_hour: 99, top_error_types: ["provider"] },
      }),
      nowPlaying: makeRuntimeEvent({
        signal_type: "error",
        status: "failed",
        priority: "critical",
        title: "Provider failed",
      }),
    });

    expectWorldNumbersSafe(world);
    expect(world.objects.map((item) => item.id)).toEqual([
      "tasks",
      "memory",
      "providers",
      "mcp",
      "git",
      "market",
    ]);
  });

  it("keeps malformed pulse numbers finite and clamped", () => {
    const malformedPulse = makePulse({
      tasks: {
        total: Number.NaN,
        stuck: Number.POSITIVE_INFINITY,
        abandoned: -4,
        by_status: {},
      },
      memory: {
        total: Number.POSITIVE_INFINITY,
        orphan: Number.NaN,
        stale_conflicts: Number.NEGATIVE_INFINITY,
      },
      providers: {
        defaults_ok: true,
        broken_refs: Number.NaN,
        quota_warnings: Number.POSITIVE_INFINITY,
      },
      mcp: {
        total: Number.NaN,
        failing: Number.POSITIVE_INFINITY,
        auth_expiring: -2,
      },
      customization: {
        modes: Number.NaN,
        skills: Number.POSITIVE_INFINITY,
        commands: -1,
        subagents: Number.NEGATIVE_INFINITY,
        hooks: Number.NaN,
      },
      diagnostics: {
        last_hour: Number.POSITIVE_INFINITY,
        top_error_types: ["generic"],
      },
      git: {
        uncommitted_files: Number.POSITIVE_INFINITY,
        diff_lines_4h: Number.NaN,
        branches: -1,
      },
    });

    expect(() => buildWorld({ pulse: malformedPulse })).not.toThrow();
    expectWorldNumbersSafe(buildWorld({ pulse: malformedPulse }));
  });

  it("keeps partial malformed pulse sections safe and neutral", () => {
    const partialPulse = {
      generated_at: "2024-01-01T00:00:00Z",
      tasks: { total: Number.NaN },
      diagnostics: { last_hour: 9, top_error_types: ["tool_failed"] },
    } as unknown as BuddyPulse;

    expect(() => buildWorld({ pulse: partialPulse })).not.toThrow();

    const world = buildWorld({ pulse: partialPulse });
    const providers = getProviderObject(world);

    expect(world.weather).not.toBe("storm");
    expect(world.atmosphere.serious).toBe(false);
    expect(world.atmosphere.layers).not.toContain("provider_storm");
    expect(providers).toMatchObject({
      state: "calm",
      animation: "sparkle",
    });
    expectWorldNumbersSafe(world);
  });
});
