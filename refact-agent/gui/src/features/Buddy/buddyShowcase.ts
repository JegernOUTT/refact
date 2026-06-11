import type {
  BuddyPetState,
  BuddyPulse,
  BuddyRuntimeEvent,
  BuddyScenePose,
  BuddyShowcaseKind,
  BuddyShowcasePhase,
  BuddyShowcaseRun,
  BuddyShowcaseTarget,
} from "./types";
import { isBuddyRuntimeEventVisible } from "./buddyRuntimeEvents";
import type { BuddyWorldPhase, BuddyWorldWeather } from "./buddyWorldModel";

export const BUDDY_SHOWCASE_PHASE_DURATIONS_MS: Record<
  BuddyShowcasePhase,
  number
> = {
  travel: 3800,
  anticipate: 900,
  showcase: 5200,
  react: 1700,
  cooldown: 1200,
};

export const BUDDY_SHOWCASE_INITIAL_GRACE_MS = 30_000;
export const BUDDY_SHOWCASE_IDLE_COOLDOWN_MS = 78_000;
export const BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS = 18_000;

const UINT_MAX = 4_294_967_295;
const MEMORY_RUNTIME_SIGNALS = new Set(["memory_extract", "knowledge_update"]);
const MEMORY_RUNTIME_STATUSES = new Set(["completed", "progress"]);
const STARGAZING_RUNTIME_SIGNALS = new Set([
  "generating",
  "streaming",
  "tool_used",
]);
const ACTIVE_RUNTIME_STATUSES = new Set(["started", "progress", "streaming"]);
const PROVIDER_TOPIC_PATTERNS = [
  /\bproviders?\b/,
  /\bquotas?\b/,
  /\bdefault[-_\s]?models?\b/,
  /\bprovider[-_\s]?sources?\b/,
  /\bbroken[-_\s]?(?:refs?|model[-_\s]?references?)\b/,
  /\bmodel[-_\s]?not[-_\s]?found\b/,
] as const;
const PROVIDER_PROBLEM_CUE_PATTERNS = [
  /\bquotas?\b/,
  /\bquota[-_\s]?warnings?\b/,
  /\bbroken[-_\s]?(?:refs?|model[-_\s]?references?)\b/,
  /\bmodel[-_\s]?not[-_\s]?found\b/,
  /\bfail(?:ed|ing|ure)?\b/,
  /\berrors?\b/,
  /\bproblems?\b/,
  /\bmissing\b/,
  /\binvalid\b/,
  /\brejected\b/,
  /\bunset\b/,
  /\bunconfigured\b/,
  /\bnot[-_\s]?configured\b/,
] as const;
const IDLE_SHOWCASE_BASE_WEIGHT = 18;
const IDLE_SHOWCASE_REPEAT_WEIGHT = 0.34;
const SHOWCASE_KIND_ORDER: Record<BuddyShowcaseKind, number> = {
  memory_firefly_night: 0,
  stargazing_constellation: 1,
  rain_shelter_dash: 2,
  koi_pond_watch: 3,
  campfire_story: 4,
  firefly_meadow_chase: 5,
  snow_sculpting: 6,
  leaf_storm_play: 7,
  aurora_dance: 8,
  komorebi_nap: 9,
};

export interface BuddyShowcaseDefinition {
  kind: BuddyShowcaseKind;
  targetId: string;
  targetSprite?: string;
  pose: BuddyScenePose;
  speech: (name: string) => string;
}

export const BUDDY_SHOWCASE_DEFINITIONS: Record<
  BuddyShowcaseKind,
  BuddyShowcaseDefinition
> = {
  memory_firefly_night: {
    kind: "memory_firefly_night",
    targetId: "memory",
    targetSprite: "memory_fireflies",
    pose: "meditate",
    speech: (name) =>
      `${name} gathers the memory fireflies into a soft night map.`,
  },
  stargazing_constellation: {
    kind: "stargazing_constellation",
    targetId: "providers",
    targetSprite: "observatory",
    pose: "stargaze",
    speech: (name) =>
      `${name} reads the model stars and traces a careful constellation.`,
  },
  rain_shelter_dash: {
    kind: "rain_shelter_dash",
    targetId: "home",
    pose: "look",
    speech: (name) =>
      `${name} dashes under the awning and watches the rain curtain.`,
  },
  koi_pond_watch: {
    kind: "koi_pond_watch",
    targetId: "pond",
    pose: "look",
    speech: (name) => `${name} holds a koi summit at the pond's edge.`,
  },
  campfire_story: {
    kind: "campfire_story",
    targetId: "campfire",
    pose: "meditate",
    speech: (name) => `${name} tells the embers a tiny heroic story.`,
  },
  firefly_meadow_chase: {
    kind: "firefly_meadow_chase",
    targetId: "meadow",
    pose: "pounce",
    speech: (name) => `${name} chases meadow fireflies in happy zigzags.`,
  },
  snow_sculpting: {
    kind: "snow_sculpting",
    targetId: "meadow",
    pose: "dig",
    speech: (name) => `${name} sculpts a tiny snow twin. It approves.`,
  },
  leaf_storm_play: {
    kind: "leaf_storm_play",
    targetId: "meadow",
    pose: "bounce",
    speech: (name) => `${name} leaps through a spiral of autumn leaves.`,
  },
  aurora_dance: {
    kind: "aurora_dance",
    targetId: "meadow",
    pose: "dance",
    speech: (name) => `${name} dances with the aurora ribbons.`,
  },
  komorebi_nap: {
    kind: "komorebi_nap",
    targetId: "great_tree",
    pose: "sleepy",
    speech: (name) => `${name} naps in the dappled light under the great tree.`,
  },
};

export type BuddyShowcaseChoice = BuddyShowcaseDefinition;

export interface BuddyShowcaseTargetCandidate extends BuddyShowcaseTarget {
  sprite?: string;
}

export interface BuddyShowcaseWorldContext {
  phase: BuddyWorldPhase;
  weather: BuddyWorldWeather;
  layers?: readonly string[];
  season?: "spring" | "summer" | "autumn" | "winter";
}

export interface ChooseBuddyShowcaseArgs {
  targets: BuddyShowcaseTargetCandidate[];
  nowPlaying: BuddyRuntimeEvent | null;
  activeSpeechVisible: boolean;
  pet: BuddyPetState | undefined;
  nowMs: number;
  idleCooldownUntilMs?: number;
  runtimeCooldownUntilMs?: number;
  idleGraceUntilMs?: number;
  lastShowcaseKind?: BuddyShowcaseKind | null;
  runtimeShowcaseEventIds?: readonly string[];
  strongRuntimeTrigger?: boolean;
  identityName?: string;
  world?: BuddyShowcaseWorldContext;
  pulse?: BuddyPulse | null;
}

export interface CreateBuddyShowcaseRunArgs extends ChooseBuddyShowcaseArgs {
  idPrefix?: string;
}

export interface AdvanceBuddyShowcasePhaseArgs {
  run: BuddyShowcaseRun;
  nowMs: number;
}

function hasProviderTopic(event: BuddyRuntimeEvent): boolean {
  const haystack = [
    event.signal_type,
    event.title,
    event.description ?? "",
    event.source,
  ]
    .join(" ")
    .toLowerCase();
  return PROVIDER_TOPIC_PATTERNS.some((pattern) => pattern.test(haystack));
}

function hasProviderProblemCue(event: BuddyRuntimeEvent): boolean {
  const haystack = [event.title, event.description ?? ""]
    .join(" ")
    .toLowerCase()
    .replace(
      /\bno[-_\s]+(?:errors?[-_\s]+or[-_\s]+warnings?|errors?|problems?|warnings?|missing[-_\s]+references?)\b/g,
      "",
    );
  return PROVIDER_PROBLEM_CUE_PATTERNS.some((pattern) =>
    pattern.test(haystack),
  );
}

function hasProviderSignal(event: BuddyRuntimeEvent | null): boolean {
  if (!event || !hasProviderTopic(event)) return false;
  const hasProblemCue = hasProviderProblemCue(event);
  if (!hasProblemCue) return false;
  if (event.status === "completed") return event.priority !== "critical";
  return true;
}

function hasProviderPulseIssue(pulse: BuddyPulse | null | undefined): boolean {
  if (!pulse) return false;
  return (
    !pulse.providers.defaults_ok ||
    pulse.providers.broken_refs > 0 ||
    pulse.providers.quota_warnings > 0
  );
}

function memoryPulseScore(pulse: BuddyPulse | null | undefined): number {
  if (!pulse) return 0;
  return (
    (pulse.memory.total > 0 ? 14 : 0) +
    pulse.memory.orphan * 6 +
    pulse.memory.stale_conflicts * 8
  );
}

function providerPulseScore(pulse: BuddyPulse | null | undefined): number {
  if (!pulse || !hasProviderPulseIssue(pulse)) return 0;
  const providers = pulse.providers;
  return (
    (!providers.defaults_ok ? 42 : 0) +
    providers.broken_refs * 16 +
    providers.quota_warnings * 12
  );
}

function kindForRuntime(
  event: BuddyRuntimeEvent | null,
  nowMs: number,
): BuddyShowcaseKind | null {
  if (event === null) return null;
  const providerSignal =
    event.dismissed === true ? false : hasProviderSignal(event);
  if (!isBuddyRuntimeEventVisible(event, nowMs) && !providerSignal) return null;
  const providerKind = providerSignal ? "stargazing_constellation" : null;
  if (providerKind && event.status === "failed") return providerKind;
  if (MEMORY_RUNTIME_SIGNALS.has(event.signal_type)) {
    return MEMORY_RUNTIME_STATUSES.has(event.status)
      ? "memory_firefly_night"
      : providerKind;
  }
  if (STARGAZING_RUNTIME_SIGNALS.has(event.signal_type)) {
    return ACTIVE_RUNTIME_STATUSES.has(event.status)
      ? "stargazing_constellation"
      : providerKind;
  }
  return providerKind;
}

export function hasBuddyShowcaseRuntimeTrigger(
  event: BuddyRuntimeEvent | null,
  nowMs = Date.now(),
): boolean {
  return kindForRuntime(event, nowMs) !== null;
}

function findTarget(
  targets: BuddyShowcaseTargetCandidate[],
  definition: BuddyShowcaseDefinition,
): BuddyShowcaseTargetCandidate | null {
  const idTarget = targets.find((target) => target.id === definition.targetId);
  if (idTarget) return idTarget;
  if (!definition.targetSprite) return null;
  return (
    targets.find((target) => target.sprite === definition.targetSprite) ?? null
  );
}

function canChooseShowcase(args: ChooseBuddyShowcaseArgs): boolean {
  if (args.activeSpeechVisible) return false;
  if (args.pet?.condition.sleeping) return false;
  const cooldownUntilMs = args.strongRuntimeTrigger
    ? args.runtimeCooldownUntilMs
    : args.idleCooldownUntilMs;
  if (args.nowMs < (cooldownUntilMs ?? 0)) return false;
  if (!args.strongRuntimeTrigger && args.nowMs < (args.idleGraceUntilMs ?? 0)) {
    return false;
  }
  return true;
}

function worldScore(
  world: BuddyShowcaseWorldContext | undefined,
  kind: BuddyShowcaseKind,
): number {
  if (!world) return 0;

  let score = 0;
  if (world.phase === "night" || world.phase === "evening") {
    score += 16;
    if (kind === "stargazing_constellation") score += 4;
    if (kind === "memory_firefly_night" && world.phase === "night") {
      score += 3;
    }
  }

  switch (world.weather) {
    case "rain":
      return score + (kind === "memory_firefly_night" ? 26 : 0);
    case "storm":
      return score + (kind === "stargazing_constellation" ? 26 : 0);
    case "aurora":
      return score + (kind === "stargazing_constellation" ? 18 : 0);
    case "busy":
      return score + (kind === "stargazing_constellation" ? 10 : 0);
    case "dream":
      return score + (kind === "memory_firefly_night" ? 8 : 0);
    case "clear":
    case "wind":
      return score;
  }
}

function hasContextLayer(
  world: BuddyShowcaseWorldContext | undefined,
  layer: string,
): boolean {
  return world?.layers?.includes(layer) ?? false;
}

function kindEligible(
  world: BuddyShowcaseWorldContext | undefined,
  kind: BuddyShowcaseKind,
): boolean {
  switch (kind) {
    case "memory_firefly_night":
    case "stargazing_constellation":
      return true;
    case "rain_shelter_dash":
      return world?.weather === "rain";
    case "koi_pond_watch":
      return hasContextLayer(world, "pond_life");
    case "campfire_story":
      return hasContextLayer(world, "campfire");
    case "firefly_meadow_chase":
      return hasContextLayer(world, "fireflies");
    case "snow_sculpting":
      return hasContextLayer(world, "season_snow");
    case "leaf_storm_play":
      return hasContextLayer(world, "season_leaves");
    case "aurora_dance":
      return hasContextLayer(world, "aurora");
    case "komorebi_nap":
      return (
        (world?.phase === "morning" || world?.phase === "day") &&
        world.weather === "clear" &&
        world.season !== undefined &&
        world.season !== "winter"
      );
  }
}

function flavorScore(
  world: BuddyShowcaseWorldContext | undefined,
  kind: BuddyShowcaseKind,
): number {
  if (!world) return 0;
  switch (kind) {
    case "rain_shelter_dash":
      return 20;
    case "koi_pond_watch":
      return world.phase === "day" ? 12 : 8;
    case "campfire_story":
      return 14;
    case "firefly_meadow_chase":
      return world.season === "summer" ? 14 : 10;
    case "snow_sculpting":
      return 12;
    case "leaf_storm_play":
      return world.weather === "wind" ? 14 : 10;
    case "aurora_dance":
      return 16;
    case "komorebi_nap":
      return 8;
    case "memory_firefly_night":
    case "stargazing_constellation":
      return 0;
  }
}

function scoreDefinition(
  args: ChooseBuddyShowcaseArgs,
  definition: BuddyShowcaseDefinition,
): number {
  let score =
    worldScore(args.world, definition.kind) +
    flavorScore(args.world, definition.kind);
  if (definition.kind === "memory_firefly_night") {
    score += memoryPulseScore(args.pulse);
  }
  if (definition.kind === "stargazing_constellation") {
    score += providerPulseScore(args.pulse);
  }
  return score;
}

function chooseWeightedDefinition(
  args: ChooseBuddyShowcaseArgs,
): BuddyShowcaseDefinition | null {
  const bucket = Math.floor(args.nowMs / BUDDY_SHOWCASE_IDLE_COOLDOWN_MS);
  const orderSeed = seedFromText(
    `${bucket}:${args.world?.phase ?? "none"}:${args.world?.weather ?? "none"}`,
  );
  const targetDefinitions = Object.values(BUDDY_SHOWCASE_DEFINITIONS).filter(
    (definition) =>
      kindEligible(args.world, definition.kind) &&
      findTarget(args.targets, definition),
  );
  const eligibleDefinitions =
    args.lastShowcaseKind && targetDefinitions.length > 1
      ? targetDefinitions.filter(
          (definition) => definition.kind !== args.lastShowcaseKind,
        )
      : targetDefinitions;
  const candidates = eligibleDefinitions
    .map((definition) => {
      const score = scoreDefinition(args, definition);
      return {
        definition,
        weight:
          Math.max(1, IDLE_SHOWCASE_BASE_WEIGHT + score) *
          (definition.kind === args.lastShowcaseKind
            ? IDLE_SHOWCASE_REPEAT_WEIGHT
            : 1),
      };
    })
    .sort((a, b) => {
      const left = seededUnit(
        orderSeed,
        SHOWCASE_KIND_ORDER[a.definition.kind],
      );
      const right = seededUnit(
        orderSeed,
        SHOWCASE_KIND_ORDER[b.definition.kind],
      );
      const seededDiff = left - right;
      if (seededDiff !== 0) return seededDiff;
      return (
        SHOWCASE_KIND_ORDER[a.definition.kind] -
        SHOWCASE_KIND_ORDER[b.definition.kind]
      );
    });

  const totalWeight = candidates.reduce(
    (total, candidate) => total + candidate.weight,
    0,
  );
  if (totalWeight <= 0) return null;

  const pulseKey = args.pulse
    ? `${args.pulse.memory.total}:${args.pulse.memory.orphan}:${args.pulse.memory.stale_conflicts}:${args.pulse.providers.broken_refs}:${args.pulse.providers.quota_warnings}:${args.pulse.providers.defaults_ok}`
    : "no-pulse";
  const rollSeed = seedFromText(
    `${bucket}:${args.lastShowcaseKind ?? "none"}:${
      args.world?.phase ?? "none"
    }:${args.world?.weather ?? "none"}:${pulseKey}`,
  );
  const roll = seededUnit(rollSeed, 19) * totalWeight;
  let cursor = 0;
  for (const candidate of candidates) {
    cursor += candidate.weight;
    if (roll <= cursor) return candidate.definition;
  }

  return candidates[candidates.length - 1]?.definition ?? null;
}

export function chooseBuddyShowcase(
  args: ChooseBuddyShowcaseArgs,
): BuddyShowcaseChoice | null {
  if (!canChooseShowcase(args)) return null;

  const runtimeKind = args.strongRuntimeTrigger
    ? kindForRuntime(args.nowPlaying, args.nowMs)
    : null;
  if (runtimeKind) {
    if (
      args.nowPlaying?.id &&
      args.runtimeShowcaseEventIds?.includes(args.nowPlaying.id)
    ) {
      return null;
    }
    const definition = BUDDY_SHOWCASE_DEFINITIONS[runtimeKind];
    return findTarget(args.targets, definition) ? definition : null;
  }

  if (args.strongRuntimeTrigger) return null;

  return chooseWeightedDefinition(args);
}

function seedFromText(text: string): number {
  let hash = 2166136261;
  for (let index = 0; index < text.length; index += 1) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function seededUnit(seed: number, salt: number): number {
  let value = (seed + Math.imul(salt + 1, 0x9e3779b9)) >>> 0;
  value ^= value >>> 16;
  value = Math.imul(value, 0x85ebca6b) >>> 0;
  value ^= value >>> 13;
  value = Math.imul(value, 0xc2b2ae35) >>> 0;
  value ^= value >>> 16;
  return (value >>> 0) / UINT_MAX;
}

export function createBuddyShowcaseSeed(args: {
  kind: BuddyShowcaseKind;
  nowMs: number;
  target: BuddyShowcaseTarget;
}): number {
  const bucketMs = Math.floor(args.nowMs / 1000);
  return seedFromText(
    `${args.kind}:${bucketMs}:${args.target.id}:${args.target.x}:${args.target.y}`,
  );
}

export function createBuddyShowcaseRun(
  args: CreateBuddyShowcaseRunArgs,
): BuddyShowcaseRun | null {
  const definition = chooseBuddyShowcase(args);
  if (!definition) return null;

  const target = findTarget(args.targets, definition);
  if (!target) return null;

  const seed = createBuddyShowcaseSeed({
    kind: definition.kind,
    nowMs: args.nowMs,
    target,
  });
  const idPrefix = args.idPrefix ?? "showcase";

  return {
    id: `${idPrefix}-${definition.kind}-${seed.toString(36)}`,
    kind: definition.kind,
    phase: "travel",
    target: {
      id: target.id,
      x: target.x,
      y: target.y,
      label: target.label,
    },
    pose: definition.pose,
    speech: definition.speech(args.identityName ?? ""),
    seed,
    startedAtMs: args.nowMs,
    phaseStartedAtMs: args.nowMs,
  };
}

function nextPhase(phase: BuddyShowcasePhase): BuddyShowcasePhase | null {
  switch (phase) {
    case "travel":
      return "anticipate";
    case "anticipate":
      return "showcase";
    case "showcase":
      return "react";
    case "react":
      return "cooldown";
    case "cooldown":
      return null;
  }
}

export function advanceBuddyShowcasePhase(
  args: AdvanceBuddyShowcasePhaseArgs,
): BuddyShowcaseRun | null {
  const elapsedMs = args.nowMs - args.run.phaseStartedAtMs;
  if (elapsedMs < BUDDY_SHOWCASE_PHASE_DURATIONS_MS[args.run.phase]) {
    return args.run;
  }

  const phase = nextPhase(args.run.phase);
  if (!phase) return null;

  return {
    ...args.run,
    phase,
    phaseStartedAtMs: args.nowMs,
  };
}
