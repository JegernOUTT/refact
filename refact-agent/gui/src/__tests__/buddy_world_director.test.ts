import { describe, expect, it } from "vitest";
import {
  chooseBuddyWorldIntent,
  type BuddyWorldIntent,
  type BuddyWorldIntentKind,
} from "../features/Buddy/buddyWorldDirector";
import { buildBuddyWorldState } from "../features/Buddy/buddyWorldModel";
import type {
  BuddyPetState,
  BuddyPulse,
  BuddyQuest,
  BuddyRuntimeEvent,
  BuddyScenePose,
} from "../features/Buddy/types";

const VALID_POSES: readonly BuddyScenePose[] = [
  "idle",
  "spin",
  "bounce",
  "look",
  "stargaze",
  "meditate",
  "pounce",
  "dance",
  "shield",
  "cheer",
  "carry",
  "dig",
  "sleepy",
];

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
    signal_type: "tool_used",
    title: "Running browser checks",
    source: "browser",
    status: "progress",
    priority: "normal",
    created_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

function buildIntent(args?: {
  hour?: number;
  pulse?: BuddyPulse | null;
  pet?: BuddyPetState;
  nowPlaying?: BuddyRuntimeEvent | null;
  previousIntent?: BuddyWorldIntent | null;
  activeSpeechVisible?: boolean;
  showcaseActive?: boolean;
  localReactionVisible?: boolean;
  reducedMotion?: boolean;
  recentIntentKinds?: readonly BuddyWorldIntentKind[];
  activeQuest?: BuddyQuest | null;
}): BuddyWorldIntent | null {
  const now = new Date(2024, 0, 1, args?.hour ?? 14, 0, 0);
  const world = buildBuddyWorldState({
    now,
    pulse: args && "pulse" in args ? args.pulse : makePulse(),
    pet: args?.pet ?? makePet(),
    nowPlaying: args?.nowPlaying ?? null,
    activeQuest: args?.activeQuest ?? null,
  });
  return chooseBuddyWorldIntent({
    world,
    previousIntent: args?.previousIntent ?? null,
    nowMs: now.getTime(),
    activeSpeechVisible: args?.activeSpeechVisible ?? false,
    showcaseActive: args?.showcaseActive ?? false,
    localReactionVisible: args?.localReactionVisible ?? false,
    reducedMotion: args?.reducedMotion ?? false,
    recentIntentKinds: args?.recentIntentKinds,
  });
}

function makePreviousIntent(
  kind: BuddyWorldIntentKind,
  priority = 100,
): BuddyWorldIntent {
  return {
    id: `previous-${kind}`,
    kind,
    targetX: 50,
    targetY: 76,
    depthScale: 1,
    pose: "idle",
    speech: null,
    speechKind: "charm",
    durationMs: 8_000,
    priority,
  };
}

function expectSafeIntent(intent: BuddyWorldIntent | null): void {
  expect(intent).not.toBeNull();
  if (!intent) return;
  expect(Number.isFinite(intent.targetX)).toBe(true);
  expect(Number.isFinite(intent.targetY)).toBe(true);
  expect(Number.isFinite(intent.depthScale)).toBe(true);
  expect(intent.targetX).toBeGreaterThanOrEqual(33);
  expect(intent.targetX).toBeLessThanOrEqual(67);
  expect(intent.targetY).toBeGreaterThanOrEqual(58);
  expect(intent.targetY).toBeLessThanOrEqual(84);
  expect(intent.depthScale).toBeGreaterThanOrEqual(0.7);
  expect(intent.depthScale).toBeLessThanOrEqual(1.2);
  expect(VALID_POSES).toContain(intent.pose);
}

describe("buddy world director", () => {
  it("channels active generic runtime at the spellforge", () => {
    const intent = buildIntent({
      nowPlaying: makeRuntimeEvent({
        signal_type: "tool_used",
        title: "Running browser checks",
        source: "browser",
        status: "progress",
      }),
    });

    expect(intent).toMatchObject({
      kind: "channel_runtime",
      targetX: 54,
    });
    expect(intent?.objectId).toBeUndefined();
    expect(intent?.speech).toBe("I’m feeding the little spellforge.");
    expectSafeIntent(intent);
  });

  it("uses the observatory only for generation/provider runtime", () => {
    const intent = buildIntent({
      nowPlaying: makeRuntimeEvent({
        signal_type: "streaming",
        title: "Streaming answer",
        source: "chat",
        status: "streaming",
      }),
    });

    expect(intent).toMatchObject({
      kind: "channel_runtime",
      objectId: "providers",
      targetX: 67,
      targetY: 74,
    });
    expectSafeIntent(intent);
  });

  it("stabilizes critical provider storms with actionable speech", () => {
    const intent = buildIntent({
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 1, quota_warnings: 0 },
      }),
    });

    expect(["stabilize_crystal", "inspect_provider"]).toContain(intent?.kind);
    expect(intent?.speechKind).toBe("actionable");
    expect(intent?.objectId).toBe("providers");
    expect(intent?.speech).toMatch(/crystal|observatory|model stars/u);
    expectSafeIntent(intent);
  });

  it("chooses memory routines for memory attention and active memory", () => {
    const attentionIntent = buildIntent({
      pulse: makePulse({
        memory: { total: 40, orphan: 3, stale_conflicts: 1 },
      }),
    });
    const activeIntent = buildIntent({
      nowPlaying: makeRuntimeEvent({
        signal_type: "memory_extract",
        title: "Gathering memory sparks",
        status: "progress",
      }),
    });

    expect(["inspect_memory", "shelve_memory"]).toContain(
      attentionIntent?.kind,
    );
    expect(["inspect_memory", "shelve_memory"]).toContain(activeIntent?.kind);
    expect(attentionIntent?.objectId).toBe("memory");
    expect(activeIntent?.objectId).toBe("memory");
    expectSafeIntent(attentionIntent);
    expectSafeIntent(activeIntent);
  });

  it("keeps generic diagnostics with healthy providers out of provider stabilization", () => {
    const intent = buildIntent({
      pulse: makePulse({
        diagnostics: {
          last_hour: 8,
          top_error_types: ["tool_failed", "browser_failure"],
        },
      }),
    });

    expect(intent?.kind).toBe("channel_runtime");
    expect(intent?.objectId).toBeUndefined();
    expect(intent?.kind).not.toBe("stabilize_crystal");
    expect(intent?.kind).not.toBe("inspect_provider");
    expectSafeIntent(intent);
  });

  it("keeps unrelated failed runtime out of provider stabilization", () => {
    const intent = buildIntent({
      nowPlaying: makeRuntimeEvent({
        signal_type: "tool_failed",
        status: "failed",
        priority: "high",
        title: "Browser action failed",
        description: "The page button was not found",
        source: "browser",
      }),
    });

    expect(intent?.kind).not.toBe("stabilize_crystal");
    expect(intent?.kind).not.toBe("inspect_provider");
    expect(intent?.objectId).not.toBe("providers");
    expectSafeIntent(intent);
  });

  it("rests at home when Buddy is sleeping", () => {
    const intent = buildIntent({
      pet: makePet({ condition: { sleeping: true } }),
    });

    expect(intent).toMatchObject({ kind: "rest_home", pose: "sleepy" });
    expectSafeIntent(intent);
  });

  it("seeks toys when Buddy is bored", () => {
    const intent = buildIntent({
      pet: makePet({ condition: { bored: true } }),
    });

    expect(intent).toMatchObject({ kind: "seek_toy" });
    expect(intent?.speech).toBe(
      "The toy nook is making mysterious eye contact.",
    );
    expectSafeIntent(intent);
  });

  it("uses ambient flavor or time routines while idle", () => {
    const dayIntent = buildIntent();
    const morningIntent = buildIntent({ hour: 8 });
    const wanderIntent = buildIntent({
      recentIntentKinds: ["play_in_snow"],
    });

    expect(dayIntent?.kind).toBe("play_in_snow");
    expect(morningIntent?.kind).toBe("morning_stretch");
    expect(wanderIntent?.kind).toBe("wander_curiously");
    expectSafeIntent(dayIntent);
    expectSafeIntent(morningIntent);
    expectSafeIntent(wanderIntent);
  });

  it("suppresses intent while active speech or showcase is visible", () => {
    expect(buildIntent({ activeSpeechVisible: true })).toBeNull();
    expect(buildIntent({ showcaseActive: true })).toBeNull();
  });

  it("continues persistent provider storms when previous provider intent matches", () => {
    const intent = buildIntent({
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 1, quota_warnings: 0 },
      }),
      previousIntent: makePreviousIntent("stabilize_crystal"),
      recentIntentKinds: ["stabilize_crystal", "inspect_provider"],
    });

    expect(intent?.kind).toBe("stabilize_crystal");
    expect(intent?.objectId).toBe("providers");
    expectSafeIntent(intent);
  });

  it("keeps persistent provider storms provider-related when recent kinds block candidates", () => {
    const intent = buildIntent({
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 1, quota_warnings: 0 },
      }),
      previousIntent: makePreviousIntent("seek_toy"),
      recentIntentKinds: [
        "stabilize_crystal",
        "inspect_provider",
        "wander_curiously",
        "watch_observatory",
      ],
    });

    expect(["stabilize_crystal", "inspect_provider"]).toContain(intent?.kind);
    expect(intent?.objectId).toBe("providers");
    expectSafeIntent(intent);
  });

  it("keeps blocked critical provider storms ahead of competing memory and runtime work", () => {
    const intent = buildIntent({
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 1, quota_warnings: 0 },
        memory: { total: 40, orphan: 4, stale_conflicts: 1 },
      }),
      nowPlaying: makeRuntimeEvent({
        signal_type: "tool_used",
        title: "Running browser checks",
        source: "browser",
        status: "progress",
      }),
      previousIntent: makePreviousIntent("seek_toy"),
      recentIntentKinds: ["stabilize_crystal", "inspect_provider"],
    });

    expect(["stabilize_crystal", "inspect_provider"]).toContain(intent?.kind);
    expect(intent?.objectId).toBe("providers");
    expectSafeIntent(intent);
  });

  it("does not let a recent high-priority candidate bypass cooldown without matching previous intent", () => {
    const intent = buildIntent({
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 1, quota_warnings: 0 },
      }),
      previousIntent: makePreviousIntent("inspect_memory"),
      recentIntentKinds: ["stabilize_crystal"],
    });

    expect(intent?.kind).toBe("inspect_provider");
    expect(intent?.objectId).toBe("providers");
    expectSafeIntent(intent);
  });

  it("suppresses recently used medium care intents", () => {
    const intent = buildIntent({
      pet: makePet({ condition: { hungry: true } }),
      recentIntentKinds: ["seek_food"],
    });

    expect(intent?.kind).toBe("play_in_snow");
    expectSafeIntent(intent);
  });

  it("suppresses duplicate low-priority idle intent kinds", () => {
    const idleSuppressed = buildIntent({
      recentIntentKinds: ["play_in_snow", "wander_curiously"],
    });
    const allIdleSuppressed = buildIntent({
      recentIntentKinds: [
        "play_in_snow",
        "wander_curiously",
        "watch_observatory",
      ],
    });

    expect(idleSuppressed?.kind).toBe("watch_observatory");
    expect(allIdleSuppressed).toBeNull();
  });

  it("celebrates recovery from previous serious intent", () => {
    const previousIntent = buildIntent({
      pulse: makePulse({
        providers: { defaults_ok: true, broken_refs: 1, quota_warnings: 0 },
      }),
    });
    const recoveredIntent = buildIntent({ previousIntent });

    expect(previousIntent?.kind).toBe("stabilize_crystal");
    expect(recoveredIntent?.kind).toBe("celebrate_recovery");
    expectSafeIntent(recoveredIntent);
  });

  it("keeps reduced motion calmer and targets safe", () => {
    const intent = buildIntent({
      reducedMotion: true,
      pet: makePet({ condition: { bored: true } }),
    });

    expect(intent).toMatchObject({ kind: "seek_toy", pose: "idle" });
    expectSafeIntent(intent);
  });

  it("checks the quest mailbox when a quest is active", () => {
    const quest: BuddyQuest = {
      id: "q1",
      quest_type: "daily",
      title: "Tidy the grove",
      description: "Close three stuck tasks",
      icon: "🌱",
      created_at: "2024-01-01T00:00:00Z",
      accepted_at: "2024-01-01T00:00:00Z",
      status: "active",
      progress: 0,
      goal: 3,
      baseline: 0,
      reward_xp: 10,
      controls: [],
    };
    const intent = buildIntent({ activeQuest: quest });

    expect(intent).toMatchObject({
      kind: "check_mailbox",
      speechKind: "actionable",
    });
    expectSafeIntent(intent);
  });

  it("offers cozy night flavor like the campfire after night watch", () => {
    const intent = buildIntent({
      hour: 23,
      recentIntentKinds: ["night_watch"],
    });

    expect(intent?.kind).toBe("warm_by_fire");
    expectSafeIntent(intent);
  });
});

describe("buddy world director seasonal flavor", () => {
  it("splashes puddles when rain falls over a calm grove", () => {
    const now = new Date(2024, 0, 1, 14, 0, 0);
    const world = buildBuddyWorldState({
      now,
      pulse: makePulse(),
      pet: makePet(),
      nowPlaying: null,
      activeQuest: null,
    });
    const intent = chooseBuddyWorldIntent({
      world: { ...world, weather: "rain" },
      previousIntent: null,
      nowMs: now.getTime(),
      activeSpeechVisible: false,
      showcaseActive: false,
      localReactionVisible: false,
      reducedMotion: false,
    });

    expect(intent?.kind).toBe("splash_puddles");
    expect(intent?.pose).toBe("bounce");
    expectSafeIntent(intent);
  });

  it("naps under the great tree on lush spring days", () => {
    const now = new Date(2024, 3, 1, 14, 0, 0);
    const world = buildBuddyWorldState({
      now,
      pulse: makePulse(),
      pet: makePet(),
      nowPlaying: null,
      activeQuest: null,
    });
    const intent = chooseBuddyWorldIntent({
      world,
      previousIntent: null,
      nowMs: now.getTime(),
      activeSpeechVisible: false,
      showcaseActive: false,
      localReactionVisible: false,
      reducedMotion: false,
      recentIntentKinds: [
        "smell_flowers",
        "chase_butterfly",
        "watch_birds",
        "visit_pond",
      ],
    });

    expect(intent?.kind).toBe("nap_under_tree");
    expect(intent?.pose).toBe("sleepy");
    expectSafeIntent(intent);
  });

  it("greets kodama at night after cozy routines rest", () => {
    const intent = buildIntent({
      hour: 23,
      recentIntentKinds: [
        "night_watch",
        "warm_by_fire",
        "watch_shooting_star",
        "play_in_snow",
      ],
    });

    expect(intent?.kind).toBe("greet_kodama");
    expectSafeIntent(intent);
  });

  it("chases soot sprites once the kodama greeting rests", () => {
    const intent = buildIntent({
      hour: 23,
      recentIntentKinds: [
        "night_watch",
        "warm_by_fire",
        "watch_shooting_star",
        "play_in_snow",
        "greet_kodama",
      ],
    });

    expect(intent?.kind).toBe("chase_soot_sprites");
    expect(intent?.pose).toBe("pounce");
    expectSafeIntent(intent);
  });
});

describe("buddy world director long activities", () => {
  function buildSummerIntent(
    recentIntentKinds: readonly BuddyWorldIntentKind[],
  ): BuddyWorldIntent | null {
    const now = new Date(2024, 6, 1, 14, 0, 0);
    const world = buildBuddyWorldState({
      now,
      pulse: makePulse(),
      pet: makePet(),
      nowPlaying: null,
      activeQuest: null,
    });
    return chooseBuddyWorldIntent({
      world,
      previousIntent: null,
      nowMs: now.getTime(),
      activeSpeechVisible: false,
      showcaseActive: false,
      localReactionVisible: false,
      reducedMotion: false,
      recentIntentKinds,
    });
  }

  const SUMMER_BASE_RECENTS: BuddyWorldIntentKind[] = [
    "chase_butterfly",
    "watch_birds",
    "visit_pond",
    "nap_under_tree",
  ];

  it("goes fishing at the pond once shorter pond visits rest", () => {
    const intent = buildSummerIntent(SUMMER_BASE_RECENTS);

    expect(intent?.kind).toBe("fish_at_pond");
    expect(intent?.pose).toBe("look");
    expect(intent?.durationMs).toBeGreaterThanOrEqual(14_000);
    expectSafeIntent(intent);
  });

  it("stacks a stone cairn after fishing rests", () => {
    const intent = buildSummerIntent([...SUMMER_BASE_RECENTS, "fish_at_pond"]);

    expect(intent?.kind).toBe("build_cairn");
    expect(intent?.pose).toBe("dig");
    expectSafeIntent(intent);
  });

  it("paints the meadow and picnics as deeper rotation options", () => {
    const paintIntent = buildSummerIntent([
      ...SUMMER_BASE_RECENTS,
      "fish_at_pond",
      "build_cairn",
    ]);
    const picnicIntent = buildSummerIntent([
      ...SUMMER_BASE_RECENTS,
      "fish_at_pond",
      "build_cairn",
      "paint_meadow",
    ]);

    expect(paintIntent?.kind).toBe("paint_meadow");
    expect(picnicIntent?.kind).toBe("picnic_snack");
    expectSafeIntent(paintIntent);
    expectSafeIntent(picnicIntent);
  });

  it("collects fireflies in a jar once cozy night routines rest", () => {
    const intent = buildIntent({
      hour: 23,
      recentIntentKinds: [
        "night_watch",
        "warm_by_fire",
        "watch_shooting_star",
        "play_in_snow",
        "greet_kodama",
        "chase_soot_sprites",
      ],
    });

    expect(intent?.kind).toBe("catch_fireflies");
    expect(intent?.pose).toBe("pounce");
    expect(intent?.durationMs).toBeGreaterThanOrEqual(12_000);
    expectSafeIntent(intent);
  });

  it("keeps long activities out of winter days so idle exhaustion still rests", () => {
    const intent = buildIntent({
      recentIntentKinds: [
        "play_in_snow",
        "wander_curiously",
        "watch_observatory",
      ],
    });

    expect(intent).toBeNull();
  });
});

describe("buddy world director totoro flavor", () => {
  function buildSeasonIntent(
    now: Date,
    recentIntentKinds: readonly BuddyWorldIntentKind[],
    weather?: "rain",
  ): BuddyWorldIntent | null {
    const world = buildBuddyWorldState({
      now,
      pulse: makePulse(),
      pet: makePet(),
      nowPlaying: null,
      activeQuest: null,
    });
    return chooseBuddyWorldIntent({
      world: weather ? { ...world, weather } : world,
      previousIntent: null,
      nowMs: now.getTime(),
      activeSpeechVisible: false,
      showcaseActive: false,
      localReactionVisible: false,
      reducedMotion: false,
      recentIntentKinds,
    });
  }

  it("gathers acorns under the great tree on autumn days", () => {
    const intent = buildSeasonIntent(new Date(2024, 9, 1, 14, 0, 0), [
      "collect_leaves",
      "chase_butterfly",
      "watch_birds",
      "visit_pond",
      "nap_under_tree",
      "fish_at_pond",
      "build_cairn",
      "paint_meadow",
      "picnic_snack",
    ]);

    expect(intent?.kind).toBe("gather_acorns");
    expect(intent?.pose).toBe("dig");
    expect(intent?.durationMs).toBeGreaterThanOrEqual(12_000);
    expectSafeIntent(intent);
  });

  it("holds a leaf umbrella once puddle splashing rests in the rain", () => {
    const intent = buildSeasonIntent(
      new Date(2024, 6, 1, 14, 0, 0),
      [
        "splash_puddles",
        "visit_pond",
        "fish_at_pond",
        "warm_by_fire",
        "watch_birds",
        "chase_butterfly",
      ],
      "rain",
    );

    expect(intent?.kind).toBe("leaf_umbrella_rain");
    expectSafeIntent(intent);
  });

  it("plays the ocarina on summer nights after cozy routines rest", () => {
    const intent = buildSeasonIntent(new Date(2024, 6, 1, 23, 0, 0), [
      "night_watch",
      "warm_by_fire",
      "watch_shooting_star",
      "greet_kodama",
      "chase_soot_sprites",
      "catch_fireflies",
      "visit_pond",
      "fish_at_pond",
    ]);

    expect(intent?.kind).toBe("play_ocarina");
    expect(intent?.pose).toBe("meditate");
    expectSafeIntent(intent);
  });

  it("performs the seed ritual once the ocarina rests on clear summer nights", () => {
    const intent = buildSeasonIntent(new Date(2024, 6, 1, 23, 0, 0), [
      "night_watch",
      "warm_by_fire",
      "watch_shooting_star",
      "greet_kodama",
      "chase_soot_sprites",
      "catch_fireflies",
      "visit_pond",
      "fish_at_pond",
      "play_ocarina",
    ]);

    expect(intent?.kind).toBe("seed_ritual");
    expect(intent?.durationMs).toBeGreaterThanOrEqual(14_000);
    expectSafeIntent(intent);
  });

  it("spins the wooden top as the deepest calm daylight rotation", () => {
    const intent = buildSeasonIntent(new Date(2024, 6, 1, 14, 0, 0), [
      "chase_butterfly",
      "watch_birds",
      "visit_pond",
      "nap_under_tree",
      "fish_at_pond",
      "build_cairn",
      "paint_meadow",
      "picnic_snack",
    ]);

    expect(intent?.kind).toBe("spin_top");
    expect(intent?.pose).toBe("spin");
    expectSafeIntent(intent);
  });
});
