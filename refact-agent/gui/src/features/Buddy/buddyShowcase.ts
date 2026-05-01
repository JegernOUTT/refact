import type {
  BuddyPetState,
  BuddyRuntimeEvent,
  BuddyScenePose,
  BuddyShowcaseKind,
  BuddyShowcasePhase,
  BuddyShowcaseRun,
  BuddyShowcaseTarget,
} from "./types";

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

export const BUDDY_SHOWCASE_IDLE_COOLDOWN_MS = 78_000;
export const BUDDY_SHOWCASE_TRIGGER_COOLDOWN_MS = 18_000;

const MEMORY_RUNTIME_SIGNALS = new Set(["memory_extract", "knowledge_update"]);
const STARGAZING_RUNTIME_SIGNALS = new Set([
  "generating",
  "streaming",
  "tool_used",
]);
const PROVIDER_ERROR_TERMS = [
  "provider",
  "model",
  "quota",
  "defaults",
] as const;

export interface BuddyShowcaseDefinition {
  kind: BuddyShowcaseKind;
  targetId: string;
  targetSprite?: string;
  pose: BuddyScenePose;
  speech: string;
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
    speech: "Buddy gathers the memory fireflies into a soft night map.",
  },
  stargazing_constellation: {
    kind: "stargazing_constellation",
    targetId: "providers",
    targetSprite: "observatory",
    pose: "stargaze",
    speech: "Buddy reads the model stars and traces a careful constellation.",
  },
};

export type BuddyShowcaseChoice = BuddyShowcaseDefinition;

export interface BuddyShowcaseTargetCandidate extends BuddyShowcaseTarget {
  sprite?: string;
}

export interface ChooseBuddyShowcaseArgs {
  targets: BuddyShowcaseTargetCandidate[];
  nowPlaying: BuddyRuntimeEvent | null;
  activeSpeechVisible: boolean;
  pet: BuddyPetState | undefined;
  nowMs: number;
  cooldownUntilMs?: number;
  lastShowcaseKind?: BuddyShowcaseKind | null;
  strongRuntimeTrigger?: boolean;
}

export interface CreateBuddyShowcaseRunArgs extends ChooseBuddyShowcaseArgs {
  idPrefix?: string;
}

export interface AdvanceBuddyShowcasePhaseArgs {
  run: BuddyShowcaseRun;
  nowMs: number;
}

function hasProviderSignal(event: BuddyRuntimeEvent | null): boolean {
  if (!event) return false;
  const haystack = [
    event.signal_type,
    event.title,
    event.description ?? "",
    event.source,
  ]
    .join(" ")
    .toLowerCase();
  return PROVIDER_ERROR_TERMS.some((term) => haystack.includes(term));
}

function kindForRuntime(
  event: BuddyRuntimeEvent | null,
): BuddyShowcaseKind | null {
  if (!event) return null;
  if (MEMORY_RUNTIME_SIGNALS.has(event.signal_type)) {
    return "memory_firefly_night";
  }
  if (
    STARGAZING_RUNTIME_SIGNALS.has(event.signal_type) ||
    hasProviderSignal(event)
  ) {
    return "stargazing_constellation";
  }
  return null;
}

function isStrongRuntimeTrigger(
  event: BuddyRuntimeEvent | null,
  explicit: boolean | undefined,
): boolean {
  return Boolean(explicit) || kindForRuntime(event) !== null;
}

function findTarget(
  targets: BuddyShowcaseTargetCandidate[],
  definition: BuddyShowcaseDefinition,
): BuddyShowcaseTargetCandidate | null {
  return (
    targets.find((target) => target.id === definition.targetId) ??
    targets.find((target) => target.sprite === definition.targetSprite) ??
    null
  );
}

function canChooseShowcase(args: ChooseBuddyShowcaseArgs): boolean {
  if (args.activeSpeechVisible) return false;
  if (args.pet?.condition.sleeping) return false;
  return args.nowMs >= (args.cooldownUntilMs ?? 0);
}

function findFirstAvailableDefinition(
  targets: BuddyShowcaseTargetCandidate[],
): BuddyShowcaseDefinition | null {
  return (
    Object.values(BUDDY_SHOWCASE_DEFINITIONS).find((definition) =>
      findTarget(targets, definition),
    ) ?? null
  );
}

export function chooseBuddyShowcase(
  args: ChooseBuddyShowcaseArgs,
): BuddyShowcaseChoice | null {
  if (!canChooseShowcase(args)) return null;

  const runtimeKind = kindForRuntime(args.nowPlaying);
  if (runtimeKind) {
    const definition = BUDDY_SHOWCASE_DEFINITIONS[runtimeKind];
    return findTarget(args.targets, definition) ? definition : null;
  }

  if (!isStrongRuntimeTrigger(args.nowPlaying, args.strongRuntimeTrigger)) {
    const idleDefinitions = Object.values(BUDDY_SHOWCASE_DEFINITIONS).filter(
      (definition) => definition.kind !== args.lastShowcaseKind,
    );
    return (
      idleDefinitions.find((definition) =>
        findTarget(args.targets, definition),
      ) ?? null
    );
  }

  return findFirstAvailableDefinition(args.targets);
}

function seedFromText(text: string): number {
  let hash = 2166136261;
  for (let index = 0; index < text.length; index += 1) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
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
    speech: definition.speech,
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
