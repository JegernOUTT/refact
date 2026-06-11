import type { BuddyScenePose } from "./types";
import type { BuddyWorldObject, BuddyWorldState } from "./buddyWorldModel";

export type BuddyWorldIntentKind =
  | "morning_stretch"
  | "evening_tidy"
  | "night_watch"
  | "rest_home"
  | "inspect_memory"
  | "shelve_memory"
  | "inspect_provider"
  | "stabilize_crystal"
  | "channel_runtime"
  | "watch_observatory"
  | "seek_food"
  | "seek_toy"
  | "receive_affection"
  | "wander_curiously"
  | "celebrate_recovery"
  | "check_mailbox"
  | "warm_by_fire"
  | "watch_shooting_star"
  | "play_in_snow"
  | "collect_leaves"
  | "smell_flowers"
  | "tend_garden"
  | "chase_butterfly"
  | "watch_birds"
  | "visit_pond"
  | "splash_puddles"
  | "nap_under_tree"
  | "greet_kodama"
  | "chase_soot_sprites";

export interface BuddyWorldIntent {
  id: string;
  kind: BuddyWorldIntentKind;
  targetX: number;
  targetY: number;
  depthScale: number;
  pose: BuddyScenePose;
  speech: string | null;
  speechKind: "charm" | "actionable";
  durationMs: number;
  priority: number;
  objectId?: string;
}

export interface ChooseBuddyWorldIntentArgs {
  world: BuddyWorldState;
  previousIntent: BuddyWorldIntent | null;
  nowMs: number;
  activeSpeechVisible: boolean;
  showcaseActive: boolean;
  localReactionVisible: boolean;
  reducedMotion: boolean;
  recentIntentKinds?: readonly BuddyWorldIntentKind[];
}

interface IntentTarget {
  targetX: number;
  targetY: number;
  depthScale: number;
  objectId?: string;
}

const TARGET_MIN_X = 33;
const TARGET_MAX_X = 67;
const TARGET_MIN_Y = 58;
const TARGET_MAX_Y = 84;
const MIN_DEPTH_SCALE = 0.7;
const MAX_DEPTH_SCALE = 1.2;
const HIGH_PRIORITY_CONTINUATION_THRESHOLD = 70;

const SAFE_TARGETS = {
  center: { targetX: 50, targetY: 76, depthScale: 1 },
  home: { targetX: 33, targetY: 76, depthScale: 0.96 },
  workshop: { targetX: 54, targetY: 77, depthScale: 1 },
  food: { targetX: 38, targetY: 78, depthScale: 0.98 },
  toy: { targetX: 46, targetY: 78, depthScale: 1 },
  observatory: { targetX: 67, targetY: 74, depthScale: 1.02 },
  pond: { targetX: 36, targetY: 82, depthScale: 1.04 },
  garden: { targetX: 41, targetY: 79, depthScale: 1 },
  campfire: { targetX: 58, targetY: 81, depthScale: 1.05 },
  mailbox: { targetX: 35, targetY: 76, depthScale: 0.94 },
  meadow: { targetX: 47, targetY: 80, depthScale: 1.02 },
  greatTree: { targetX: 34, targetY: 78, depthScale: 0.98 },
} as const satisfies Record<string, IntentTarget>;

function clampRange(
  value: number,
  min: number,
  max: number,
  fallback: number,
): number {
  const finiteValue = Number.isFinite(value) ? value : fallback;
  return Math.max(min, Math.min(max, finiteValue));
}

function clampTarget(target: IntentTarget): IntentTarget {
  const base = {
    targetX: clampRange(target.targetX, TARGET_MIN_X, TARGET_MAX_X, 50),
    targetY: clampRange(target.targetY, TARGET_MIN_Y, TARGET_MAX_Y, 76),
    depthScale: clampRange(
      target.depthScale,
      MIN_DEPTH_SCALE,
      MAX_DEPTH_SCALE,
      1,
    ),
  };
  return target.objectId ? { ...base, objectId: target.objectId } : base;
}

function targetForObject(
  object: BuddyWorldObject | undefined,
  fallback: IntentTarget,
): IntentTarget {
  if (!object) return clampTarget(fallback);
  return clampTarget({
    targetX: object.interactionX,
    targetY: object.interactionY,
    depthScale: object.depthScale,
    objectId: object.id,
  });
}

function findObject(
  world: BuddyWorldState,
  id: string,
): BuddyWorldObject | undefined {
  return world.objects.find((object) => object.id === id);
}

function hasLayer(world: BuddyWorldState, layer: string): boolean {
  return world.atmosphere.layers.some((item) => item === layer);
}

function poseForReducedMotion(
  pose: BuddyScenePose,
  reducedMotion: boolean,
): BuddyScenePose {
  if (!reducedMotion) return pose;
  switch (pose) {
    case "spin":
    case "bounce":
    case "pounce":
    case "dance":
    case "cheer":
    case "dig":
      return "idle";
    case "shield":
      return "look";
    case "idle":
    case "look":
    case "stargaze":
    case "meditate":
    case "carry":
    case "sleepy":
      return pose;
  }
}

function intentId(kind: BuddyWorldIntentKind, nowMs: number): string {
  const bucket = Number.isFinite(nowMs)
    ? Math.max(0, Math.floor(nowMs / 1000))
    : 0;
  return `director-${kind}-${bucket.toString(36)}`;
}

function makeIntent(args: {
  kind: BuddyWorldIntentKind;
  target: IntentTarget;
  pose: BuddyScenePose;
  speech: string | null;
  speechKind?: "charm" | "actionable";
  durationMs: number;
  priority: number;
  nowMs: number;
  reducedMotion: boolean;
}): BuddyWorldIntent {
  const target = clampTarget(args.target);
  const durationMs = args.reducedMotion
    ? Math.round(args.durationMs * 1.45)
    : args.durationMs;
  const base = {
    id: intentId(args.kind, args.nowMs),
    kind: args.kind,
    targetX: target.targetX,
    targetY: target.targetY,
    depthScale: target.depthScale,
    pose: poseForReducedMotion(args.pose, args.reducedMotion),
    speech:
      args.reducedMotion && args.kind === "wander_curiously"
        ? null
        : args.speech,
    speechKind: args.speechKind ?? "charm",
    durationMs,
    priority: args.priority,
  } satisfies Omit<BuddyWorldIntent, "objectId">;
  return target.objectId ? { ...base, objectId: target.objectId } : base;
}

function isProviderIntent(kind: BuddyWorldIntentKind): boolean {
  return kind === "stabilize_crystal" || kind === "inspect_provider";
}

function isProviderRecoveryIntent(kind: BuddyWorldIntentKind): boolean {
  return isProviderIntent(kind) || kind === "watch_observatory";
}

function isMemoryIntent(kind: BuddyWorldIntentKind): boolean {
  return kind === "inspect_memory" || kind === "shelve_memory";
}

function isPersistentCriticalIntent(candidate: BuddyWorldIntent): boolean {
  if (candidate.kind === "channel_runtime") return candidate.priority >= 80;
  if (isMemoryIntent(candidate.kind)) return candidate.priority >= 80;
  return isProviderIntent(candidate.kind) && candidate.priority >= 90;
}

function canContinueRecentIntent(
  candidate: BuddyWorldIntent,
  previousIntent: BuddyWorldIntent | null,
): boolean {
  return (
    previousIntent?.kind === candidate.kind &&
    candidate.priority >= HIGH_PRIORITY_CONTINUATION_THRESHOLD
  );
}

function buildRecoveryIntent(args: {
  previousIntent: BuddyWorldIntent | null;
  providerObject: BuddyWorldObject | undefined;
  memoryObject: BuddyWorldObject | undefined;
  providerSerious: boolean;
  runtimeActive: boolean;
  nowMs: number;
  reducedMotion: boolean;
}): BuddyWorldIntent | null {
  const previousIntent = args.previousIntent;
  if (!previousIntent) return null;

  const providerRecovered =
    isProviderRecoveryIntent(previousIntent.kind) &&
    !args.providerSerious &&
    args.providerObject?.state === "calm";
  const memoryRecovered =
    isMemoryIntent(previousIntent.kind) && args.memoryObject?.state === "calm";
  const runtimeRecovered =
    previousIntent.kind === "channel_runtime" && !args.runtimeActive;

  if (!providerRecovered && !memoryRecovered && !runtimeRecovered) return null;

  const target = providerRecovered
    ? targetForObject(args.providerObject, SAFE_TARGETS.observatory)
    : memoryRecovered
      ? targetForObject(args.memoryObject, SAFE_TARGETS.center)
      : SAFE_TARGETS.workshop;

  return makeIntent({
    kind: "celebrate_recovery",
    target,
    pose: "cheer",
    speech: "Tiny recovery sparkle. Everything hums steadier now.",
    durationMs: 8_400,
    priority: 78,
    nowMs: args.nowMs,
    reducedMotion: args.reducedMotion,
  });
}

function pickIntent(
  candidates: BuddyWorldIntent[],
  recentIntentKinds: readonly BuddyWorldIntentKind[] | undefined,
  previousIntent: BuddyWorldIntent | null,
): BuddyWorldIntent | null {
  const recentKinds = new Set(recentIntentKinds ?? []);
  let blockedCriticalIntent: BuddyWorldIntent | null = null;

  for (const candidate of candidates) {
    if (
      blockedCriticalIntent &&
      candidate.priority < HIGH_PRIORITY_CONTINUATION_THRESHOLD
    ) {
      return blockedCriticalIntent;
    }

    if (
      blockedCriticalIntent &&
      isProviderIntent(blockedCriticalIntent.kind) &&
      !isProviderIntent(candidate.kind)
    ) {
      return blockedCriticalIntent;
    }

    if (!recentKinds.has(candidate.kind)) return candidate;
    if (canContinueRecentIntent(candidate, previousIntent)) return candidate;

    if (!blockedCriticalIntent && isPersistentCriticalIntent(candidate)) {
      blockedCriticalIntent = candidate;
    }
  }

  return blockedCriticalIntent;
}

export function chooseBuddyWorldIntent(
  args: ChooseBuddyWorldIntentArgs,
): BuddyWorldIntent | null {
  if (args.showcaseActive) return null;
  if (args.activeSpeechVisible) return null;

  const providerObject = findObject(args.world, "providers");
  const memoryObject = findObject(args.world, "memory");
  const providerSerious =
    hasLayer(args.world, "provider_storm") ||
    providerObject?.state === "critical";
  const providerAttention =
    !providerSerious &&
    (hasLayer(args.world, "provider_flicker") ||
      providerObject?.state === "attention");
  const memoryActive = memoryObject?.state === "active";
  const memoryAttention =
    memoryObject?.state === "attention" || memoryObject?.state === "critical";
  const runtimeActive =
    args.world.weather === "busy" ||
    args.world.atmosphere.mood === "busy" ||
    hasLayer(args.world, "workshop_runes");
  const providerRuntimeActive = providerObject?.state === "active";
  const memoryRuntimeActive = memoryActive;

  const providerTarget = targetForObject(
    providerObject,
    SAFE_TARGETS.observatory,
  );
  const memoryTarget = targetForObject(memoryObject, SAFE_TARGETS.center);
  const runtimeTarget = providerRuntimeActive
    ? providerTarget
    : memoryRuntimeActive
      ? memoryTarget
      : SAFE_TARGETS.workshop;

  const highPriorityCandidates: BuddyWorldIntent[] = [];

  if (providerSerious) {
    highPriorityCandidates.push(
      makeIntent({
        kind: "stabilize_crystal",
        target: providerTarget,
        pose: "shield",
        speech: "I’m nudging the crystal back into tune.",
        speechKind: "actionable",
        durationMs: 10_600,
        priority: 100,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
      makeIntent({
        kind: "inspect_provider",
        target: providerTarget,
        pose: "stargaze",
        speech: "The model stars are flickering; I’m checking the observatory.",
        speechKind: "actionable",
        durationMs: 10_200,
        priority: 96,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }

  if (memoryActive || memoryAttention) {
    highPriorityCandidates.push(
      makeIntent({
        kind: memoryActive ? "inspect_memory" : "shelve_memory",
        target: memoryTarget,
        pose: memoryActive ? "meditate" : "carry",
        speech: memoryActive
          ? "I’m gathering loose memory sparks."
          : "These fireflies want a shelf.",
        speechKind: memoryActive ? "charm" : "actionable",
        durationMs: memoryActive ? 9_400 : 9_800,
        priority: memoryActive ? 90 : 84,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }

  if (runtimeActive) {
    highPriorityCandidates.push(
      makeIntent({
        kind: "channel_runtime",
        target: runtimeTarget,
        pose: providerRuntimeActive ? "stargaze" : "meditate",
        speech: providerRuntimeActive
          ? "The runes are compiling something shiny."
          : "I’m feeding the little spellforge.",
        durationMs: 9_600,
        priority: providerRuntimeActive ? 88 : 82,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }

  if (providerAttention) {
    highPriorityCandidates.push(
      makeIntent({
        kind: "inspect_provider",
        target: providerTarget,
        pose: "stargaze",
        speech: "I’m checking the model stars before they grumble.",
        speechKind: "actionable",
        durationMs: 9_200,
        priority: 74,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }

  const recoveryIntent = buildRecoveryIntent({
    previousIntent: args.previousIntent,
    providerObject,
    memoryObject,
    providerSerious,
    runtimeActive,
    nowMs: args.nowMs,
    reducedMotion: args.reducedMotion,
  });

  const mediumPriorityCandidates: BuddyWorldIntent[] = [];

  switch (args.world.atmosphere.mood) {
    case "sleepy":
      mediumPriorityCandidates.push(
        makeIntent({
          kind: "rest_home",
          target: SAFE_TARGETS.home,
          pose: "sleepy",
          speech: "Dream mist accepted. I’ll keep one eye on the hearth.",
          durationMs: 12_000,
          priority: 68,
          nowMs: args.nowMs,
          reducedMotion: args.reducedMotion,
        }),
      );
      break;
    case "hungry":
      mediumPriorityCandidates.push(
        makeIntent({
          kind: "seek_food",
          target: SAFE_TARGETS.food,
          pose: "pounce",
          speech: "Snack beacon detected.",
          durationMs: 8_600,
          priority: 62,
          nowMs: args.nowMs,
          reducedMotion: args.reducedMotion,
        }),
      );
      break;
    case "bored":
      mediumPriorityCandidates.push(
        makeIntent({
          kind: "seek_toy",
          target: SAFE_TARGETS.toy,
          pose: "pounce",
          speech: "The toy nook is making mysterious eye contact.",
          durationMs: 8_600,
          priority: 60,
          nowMs: args.nowMs,
          reducedMotion: args.reducedMotion,
        }),
      );
      break;
    case "affectionate":
      mediumPriorityCandidates.push(
        makeIntent({
          kind: "receive_affection",
          target: SAFE_TARGETS.home,
          pose: "bounce",
          speech: "Pocket warmth received. I’m glowing responsibly.",
          durationMs: 8_200,
          priority: 58,
          nowMs: args.nowMs,
          reducedMotion: args.reducedMotion,
        }),
      );
      break;
    case "serene":
    case "curious":
    case "busy":
    case "unstable":
      break;
  }

  if (hasLayer(args.world, "quest_mailbox")) {
    mediumPriorityCandidates.push(
      makeIntent({
        kind: "check_mailbox",
        target: SAFE_TARGETS.mailbox,
        pose: "look",
        speech: "The quest mailbox flag is up. New orders inside!",
        speechKind: "actionable",
        durationMs: 8_400,
        priority: 56,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }

  switch (args.world.phase) {
    case "morning":
      mediumPriorityCandidates.push(
        makeIntent({
          kind: "morning_stretch",
          target: SAFE_TARGETS.center,
          pose: "bounce",
          speech: "Morning stretch. Systems: squeaky but ready.",
          durationMs: 8_800,
          priority: 42,
          nowMs: args.nowMs,
          reducedMotion: args.reducedMotion,
        }),
      );
      break;
    case "evening":
      mediumPriorityCandidates.push(
        makeIntent({
          kind: "evening_tidy",
          target: memoryTarget,
          pose: "carry",
          speech: "Evening tidy. I’m tucking stray sparks in.",
          durationMs: 8_800,
          priority: 40,
          nowMs: args.nowMs,
          reducedMotion: args.reducedMotion,
        }),
      );
      break;
    case "night":
      mediumPriorityCandidates.push(
        makeIntent({
          kind: "night_watch",
          target: SAFE_TARGETS.observatory,
          pose: "stargaze",
          speech: "Night watch mode. I’ll keep the constellations tidy.",
          durationMs: 9_200,
          priority: 38,
          nowMs: args.nowMs,
          reducedMotion: args.reducedMotion,
        }),
      );
      break;
    case "day":
      break;
  }

  const flavorCandidates: BuddyWorldIntent[] = [];

  if (args.world.weather === "rain") {
    flavorCandidates.push(
      makeIntent({
        kind: "splash_puddles",
        target: SAFE_TARGETS.meadow,
        pose: "bounce",
        speech: "Puddle physics research. Very important.",
        durationMs: 8_800,
        priority: 26,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }
  if (hasLayer(args.world, "campfire")) {
    flavorCandidates.push(
      makeIntent({
        kind: "warm_by_fire",
        target: SAFE_TARGETS.campfire,
        pose: "meditate",
        speech: "Campfire status: crackling within parameters.",
        durationMs: 10_400,
        priority: 30,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }
  if (hasLayer(args.world, "shooting_stars")) {
    flavorCandidates.push(
      makeIntent({
        kind: "watch_shooting_star",
        target: SAFE_TARGETS.observatory,
        pose: "stargaze",
        speech: "A star just zipped across the sky. Wish logged.",
        durationMs: 9_200,
        priority: 24,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }
  if (hasLayer(args.world, "season_snow")) {
    flavorCandidates.push(
      makeIntent({
        kind: "play_in_snow",
        target: SAFE_TARGETS.meadow,
        pose: "dig",
        speech: "Snow! I’m sculpting a tiny code angel.",
        durationMs: 9_000,
        priority: 22,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }
  if (hasLayer(args.world, "season_leaves")) {
    flavorCandidates.push(
      makeIntent({
        kind: "collect_leaves",
        target: SAFE_TARGETS.garden,
        pose: "carry",
        speech: "Collecting the crunchiest leaves for the archive.",
        durationMs: 8_800,
        priority: 21,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }
  if (hasLayer(args.world, "season_petals")) {
    flavorCandidates.push(
      makeIntent({
        kind: "smell_flowers",
        target: SAFE_TARGETS.garden,
        pose: "look",
        speech: "Petal report: fragrant and non-blocking.",
        durationMs: 8_600,
        priority: 21,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }
  if (
    args.world.vitality !== "lush" &&
    (args.world.phase === "morning" || args.world.phase === "day")
  ) {
    flavorCandidates.push(
      makeIntent({
        kind: "tend_garden",
        target: SAFE_TARGETS.garden,
        pose: "dig",
        speech: "Watering the task sprouts back to green.",
        durationMs: 9_000,
        priority: 20,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }
  if (hasLayer(args.world, "butterflies")) {
    flavorCandidates.push(
      makeIntent({
        kind: "chase_butterfly",
        target: SAFE_TARGETS.meadow,
        pose: "pounce",
        speech: "A butterfly! Critical chase business.",
        durationMs: 8_400,
        priority: 19,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }
  if (hasLayer(args.world, "birds")) {
    flavorCandidates.push(
      makeIntent({
        kind: "watch_birds",
        target: SAFE_TARGETS.meadow,
        pose: "look",
        speech: "Bird patrol overhead. All wings accounted for.",
        durationMs: 8_200,
        priority: 16,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }
  if (hasLayer(args.world, "pond_life")) {
    flavorCandidates.push(
      makeIntent({
        kind: "visit_pond",
        target: SAFE_TARGETS.pond,
        pose: "look",
        speech: "The koi shared confidential pond gossip.",
        durationMs: 8_400,
        priority: 14,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }
  if (
    (args.world.phase === "day" || args.world.phase === "morning") &&
    args.world.weather === "clear" &&
    args.world.season !== "winter" &&
    args.world.vitality === "lush"
  ) {
    flavorCandidates.push(
      makeIntent({
        kind: "nap_under_tree",
        target: SAFE_TARGETS.greatTree,
        pose: "sleepy",
        speech: "The leaf shade is perfect. Quick recharge nap.",
        durationMs: 11_200,
        priority: 18,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }
  if (args.world.phase === "night") {
    flavorCandidates.push(
      makeIntent({
        kind: "greet_kodama",
        target: SAFE_TARGETS.greatTree,
        pose: "look",
        speech: "The little forest spirits are out. Waving politely.",
        durationMs: 9_200,
        priority: 15,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }
  if (args.world.phase === "evening" || args.world.phase === "night") {
    flavorCandidates.push(
      makeIntent({
        kind: "chase_soot_sprites",
        target: SAFE_TARGETS.home,
        pose: "pounce",
        speech: "Soot sprites!! Tiny, fast, suspicious.",
        durationMs: 8_600,
        priority: 13,
        nowMs: args.nowMs,
        reducedMotion: args.reducedMotion,
      }),
    );
  }

  const lowPriorityCandidates = [
    makeIntent({
      kind: "wander_curiously",
      target: SAFE_TARGETS.center,
      pose: "look",
      speech: args.localReactionVisible
        ? null
        : "I’m checking the sparkle map.",
      durationMs: 8_000,
      priority: 10,
      nowMs: args.nowMs,
      reducedMotion: args.reducedMotion,
    }),
    makeIntent({
      kind: "watch_observatory",
      target: SAFE_TARGETS.observatory,
      pose: "stargaze",
      speech: args.localReactionVisible
        ? null
        : "I’m counting the quiet model stars.",
      durationMs: 8_400,
      priority: 8,
      nowMs: args.nowMs,
      reducedMotion: args.reducedMotion,
    }),
  ];

  return pickIntent(
    [
      ...highPriorityCandidates,
      ...(recoveryIntent ? [recoveryIntent] : []),
      ...mediumPriorityCandidates,
      ...flavorCandidates,
      ...lowPriorityCandidates,
    ],
    args.recentIntentKinds,
    args.previousIntent,
  );
}
