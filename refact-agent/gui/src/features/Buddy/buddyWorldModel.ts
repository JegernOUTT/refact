import { isBuddyRuntimeEventVisible } from "./buddyRuntimeEvents";
import type {
  BuddyPage,
  BuddyPetState,
  BuddyPulse,
  BuddyQuest,
  BuddyRuntimeEvent,
  BuddySemanticState,
} from "./types";

export type BuddyWorldPhase = "morning" | "day" | "evening" | "night";
export type BuddyWorldWeather =
  | "clear"
  | "aurora"
  | "busy"
  | "wind"
  | "rain"
  | "storm"
  | "dream";

export type BuddyWorldMood =
  | "serene"
  | "curious"
  | "busy"
  | "sleepy"
  | "hungry"
  | "bored"
  | "affectionate"
  | "unstable";

export type BuddyWorldLayer =
  | "sun_motes"
  | "moths"
  | "fireflies"
  | "stars"
  | "aurora"
  | "dream_mist"
  | "workshop_runes"
  | "provider_storm"
  | "provider_flicker"
  | "memory_orbs"
  | "cozy_home_glow"
  | "toy_glow"
  | "empty_food_nook";

export interface BuddyWorldAtmosphere {
  phase: BuddyWorldPhase;
  mood: BuddyWorldMood;
  primaryWeather: BuddyWorldWeather;
  layers: BuddyWorldLayer[];
  intensity: number;
  paletteHint: "dawn" | "day" | "dusk" | "night" | "dream" | "storm";
  serious: boolean;
}

export type BuddyWorldTone = "good" | "neutral" | "warning" | "danger";
export type BuddyWorldSprite =
  | "task_grove"
  | "memory_fireflies"
  | "observatory"
  | "satellite"
  | "git_vane"
  | "market_comet"
  | "seed";

export type BuddyWorldObjectState =
  | "calm"
  | "active"
  | "attention"
  | "critical"
  | "celebrating";

export type BuddyWorldObjectAnimation =
  | "none"
  | "breathe"
  | "sparkle"
  | "flicker"
  | "orbit"
  | "wobble"
  | "storm"
  | "stream"
  | "dim";

export interface BuddyWorldObject {
  id: string;
  sprite: BuddyWorldSprite;
  label: string;
  value: string;
  description: string;
  page: BuddyPage;
  tone: BuddyWorldTone;
  x: number;
  y: number;
  size: number;
  state: BuddyWorldObjectState;
  intensity: number;
  animation: BuddyWorldObjectAnimation;
  interactionX: number;
  interactionY: number;
  depthScale: number;
  magicalLabel?: string;
}

export interface BuddyWorldState {
  phase: BuddyWorldPhase;
  phaseLabel: string;
  phaseMessage: string;
  celestialEmoji: string;
  celestialLabel: string;
  celestialAction: string;
  celestialX: number;
  celestialY: number;
  weather: BuddyWorldWeather;
  weatherLabel: string;
  weatherDescription: string;
  weatherX: number;
  weatherY: number;
  vitality: "lush" | "growing" | "tangled";
  vitalityLabel: string;
  objects: BuddyWorldObject[];
  atmosphere: BuddyWorldAtmosphere;
  headline: string;
}

type BuddyWorldObjectBase = Omit<
  BuddyWorldObject,
  | "state"
  | "intensity"
  | "animation"
  | "interactionX"
  | "interactionY"
  | "depthScale"
  | "magicalLabel"
>;

type BuddyWorldObjectSemanticFields = Pick<
  BuddyWorldObject,
  | "state"
  | "intensity"
  | "animation"
  | "interactionX"
  | "interactionY"
  | "depthScale"
  | "magicalLabel"
>;

const ACTIVE_RUNTIME_STATUSES = new Set<BuddyRuntimeEvent["status"]>([
  "started",
  "progress",
  "streaming",
]);

const MEMORY_RUNTIME_SIGNALS = new Set(["memory_extract", "knowledge_update"]);

const GENERATION_RUNTIME_SIGNALS = new Set([
  "streaming",
  "generating",
  "title_generating",
]);

const PROVIDER_MODEL_TOPIC_TERMS = [
  "provider",
  "model",
  "default_model",
  "default model",
  "default_model_missing",
  "broken_ref",
  "broken ref",
  "broken_model_reference",
  "model_not_found",
  "quota",
  "llm",
  "openai",
  "anthropic",
  "claude",
  "gemini",
  "groq",
  "ollama",
  "openrouter",
  "vllm",
  "xai",
  "context window",
  "rate limit",
  "api key",
  "apikey",
] as const;

const AFFECTION_SIGNALS = new Set([
  "care_pet",
  "care_play",
  "care_feed",
  "stage_up",
  "skill_learned",
]);

function clampRange(
  value: number,
  min: number,
  max: number,
  fallback: number,
): number {
  const finiteValue = Number.isFinite(value) ? value : fallback;
  return Math.max(min, Math.min(max, finiteValue));
}

function clamp01(value: number): number {
  return clampRange(value, 0, 1, 0);
}

function safeCount(value: number | null | undefined): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return 0;
  return Math.max(0, value);
}

function addLayer(layers: BuddyWorldLayer[], layer: BuddyWorldLayer): void {
  if (!layers.includes(layer)) layers.push(layer);
}

function visibleRuntimeEvent(
  event: BuddyRuntimeEvent | null,
  nowMs: number,
): BuddyRuntimeEvent | null {
  return isBuddyRuntimeEventVisible(event, nowMs) ? event : null;
}

function isActiveRuntime(event: BuddyRuntimeEvent | null): boolean {
  return event !== null && ACTIVE_RUNTIME_STATUSES.has(event.status);
}

function runtimeText(event: BuddyRuntimeEvent): string {
  return `${event.signal_type} ${event.title} ${event.description ?? ""} ${
    event.source
  }`.toLowerCase();
}

function hasProviderModelTopicText(text: string): boolean {
  const normalized = text.toLowerCase();
  return PROVIDER_MODEL_TOPIC_TERMS.some((term) => normalized.includes(term));
}

function hasProviderModelRuntimeTopic(event: BuddyRuntimeEvent): boolean {
  return hasProviderModelTopicText(runtimeText(event));
}

function isGenerationRuntime(event: BuddyRuntimeEvent): boolean {
  return GENERATION_RUNTIME_SIGNALS.has(event.signal_type);
}

function isMemoryRuntimeActive(event: BuddyRuntimeEvent | null): boolean {
  return (
    event !== null &&
    isActiveRuntime(event) &&
    MEMORY_RUNTIME_SIGNALS.has(event.signal_type)
  );
}

function isProviderRuntimeActive(event: BuddyRuntimeEvent | null): boolean {
  if (event === null || !isActiveRuntime(event)) return false;
  return isGenerationRuntime(event) || hasProviderModelRuntimeTopic(event);
}

function isProviderModelRuntimeProblem(
  event: BuddyRuntimeEvent | null,
): boolean {
  if (event === null || event.status !== "failed") return false;
  return isGenerationRuntime(event) || hasProviderModelRuntimeTopic(event);
}

function providerWarningCount(pulse: BuddyPulse | null | undefined): number {
  if (!pulse) return 0;
  return (
    safeCount(pulse.providers.quota_warnings) +
    (pulse.providers.defaults_ok === true ? 0 : 1)
  );
}

function providerCriticalCount(pulse: BuddyPulse | null | undefined): number {
  return safeCount(pulse?.providers.broken_refs);
}

function diagnosticPressure(pulse: BuddyPulse | null | undefined): number {
  return safeCount(pulse?.diagnostics.last_hour);
}

function diagnosticsTopicText(pulse: BuddyPulse): string {
  return pulse.diagnostics.top_error_types.join(" ");
}

function hasGenericDiagnosticPressure(
  pulse: BuddyPulse | null | undefined,
): boolean {
  return diagnosticPressure(pulse) >= 6;
}

function memoryPressure(pulse: BuddyPulse | null | undefined): number {
  if (!pulse) return 0;
  return (
    safeCount(pulse.memory.orphan) + safeCount(pulse.memory.stale_conflicts) * 2
  );
}

function memoryIssueCount(pulse: BuddyPulse | null | undefined): number {
  if (!pulse) return 0;
  return (
    safeCount(pulse.memory.orphan) + safeCount(pulse.memory.stale_conflicts)
  );
}

function hasProviderModelPulseProblem(
  pulse: BuddyPulse | null | undefined,
): boolean {
  if (!pulse) return false;
  return (
    providerCriticalCount(pulse) > 0 ||
    (diagnosticPressure(pulse) >= 6 &&
      !pulse.providers.defaults_ok &&
      hasProviderModelTopicText(diagnosticsTopicText(pulse)))
  );
}

function phaseFromHour(hour: number): BuddyWorldPhase {
  if (hour >= 5 && hour < 11) return "morning";
  if (hour >= 11 && hour < 17) return "day";
  if (hour >= 17 && hour < 21) return "evening";
  return "night";
}

function phaseDetails(
  phase: BuddyWorldPhase,
): Pick<
  BuddyWorldState,
  | "phaseLabel"
  | "phaseMessage"
  | "celestialEmoji"
  | "celestialLabel"
  | "celestialAction"
  | "celestialX"
  | "celestialY"
> {
  switch (phase) {
    case "morning":
      return {
        phaseLabel: "Morning boot glow",
        phaseMessage: "The sun is warming up the project garden.",
        celestialEmoji: "🌅",
        celestialLabel: "Sunrise",
        celestialAction: "Charge focus",
        celestialX: 18,
        celestialY: 22,
      };
    case "day":
      return {
        phaseLabel: "Daylight build mode",
        phaseMessage: "Everything is bright enough for Buddy to inspect.",
        celestialEmoji: "☀️",
        celestialLabel: "Sun",
        celestialAction: "Play in sun",
        celestialX: 48,
        celestialY: 14,
      };
    case "evening":
      return {
        phaseLabel: "Evening cooldown",
        phaseMessage: "Soft light, tidy notes, one more productive pass.",
        celestialEmoji: "🌇",
        celestialLabel: "Low sun",
        celestialAction: "Gather sparks",
        celestialX: 78,
        celestialY: 26,
      };
    case "night":
      return {
        phaseLabel: "Night daemon watch",
        phaseMessage: "The moon is up and Buddy is watching quiet queues.",
        celestialEmoji: "🌙",
        celestialLabel: "Moon",
        celestialAction: "Let Buddy rest",
        celestialX: 74,
        celestialY: 16,
      };
  }
}

function toneFromCount(
  count: number,
  warnAt: number,
  dangerAt: number,
): BuddyWorldTone {
  if (count >= dangerAt) return "danger";
  if (count >= warnAt) return "warning";
  return count > 0 ? "neutral" : "good";
}

function buildWorldObject(
  base: BuddyWorldObjectBase,
  semantic: Partial<BuddyWorldObjectSemanticFields>,
): BuddyWorldObject {
  const object = {
    ...base,
    state: semantic.state ?? "calm",
    intensity: clamp01(semantic.intensity ?? 0.24),
    animation: semantic.animation ?? "breathe",
    interactionX: clampRange(semantic.interactionX ?? base.x, 0, 100, 50),
    interactionY: clampRange(
      semantic.interactionY ?? Math.max(base.y, 72),
      58,
      84,
      76,
    ),
    depthScale: clampRange(semantic.depthScale ?? 1, 0.7, 1.2, 1),
  } satisfies BuddyWorldObject;
  if (semantic.magicalLabel) {
    return { ...object, magicalLabel: semantic.magicalLabel };
  }
  return object;
}

function buildObjects(
  pulse: BuddyPulse | null | undefined,
  visibleRuntime: BuddyRuntimeEvent | null,
): BuddyWorldObject[] {
  const runtimeActive = isActiveRuntime(visibleRuntime);
  const memoryRuntimeActive = isMemoryRuntimeActive(visibleRuntime);
  const providerRuntimeActive = isProviderRuntimeActive(visibleRuntime);
  const runtimeProviderProblem = isProviderModelRuntimeProblem(visibleRuntime);

  if (!pulse) {
    return [
      buildWorldObject(
        {
          id: "warming-up",
          sprite: "seed",
          label: "Project garden",
          value: "Warming up",
          description: "Buddy is waiting for a pulse snapshot.",
          page: { type: "buddy" },
          tone: "neutral",
          x: 25,
          y: 70,
          size: 12,
        },
        {
          state: runtimeActive ? "active" : "calm",
          intensity: runtimeActive ? 0.72 : 0.32,
          animation: runtimeActive ? "stream" : "breathe",
          interactionX: 32,
          interactionY: 76,
          depthScale: 0.92,
          magicalLabel: "Sprouting hearth",
        },
      ),
    ];
  }

  const providerWarnings = providerWarningCount(pulse);
  const providerCritical =
    providerCriticalCount(pulse) > 0 || runtimeProviderProblem;
  const providerIssues = providerCriticalCount(pulse) + providerWarnings;
  const memoryIssues = memoryIssueCount(pulse);
  const memoryLoad = memoryPressure(pulse);
  const memoryCritical =
    safeCount(pulse.memory.stale_conflicts) >= 6 || memoryLoad >= 8;
  const taskTotal = safeCount(pulse.tasks.total);
  const taskStuck = safeCount(pulse.tasks.stuck);
  const taskPressure = taskStuck + safeCount(pulse.tasks.abandoned);
  const memoryTotal = safeCount(pulse.memory.total);
  const mcpFailing = safeCount(pulse.mcp.failing);
  const mcpAuthExpiring = safeCount(pulse.mcp.auth_expiring);
  const mcpPressure = mcpFailing + mcpAuthExpiring;
  const gitUncommittedFiles = safeCount(pulse.git.uncommitted_files);
  const gitDiffLines = safeCount(pulse.git.diff_lines_4h);
  const gitPressure = gitUncommittedFiles + (gitDiffLines > 0 ? 1 : 0);
  const customizationTools =
    safeCount(pulse.customization.skills) +
    safeCount(pulse.customization.commands);

  const memoryState: BuddyWorldObjectState = memoryRuntimeActive
    ? "active"
    : memoryCritical
      ? "critical"
      : memoryIssues > 0
        ? "attention"
        : "calm";
  const providerState: BuddyWorldObjectState = providerCritical
    ? "critical"
    : providerRuntimeActive
      ? "active"
      : providerWarnings > 0
        ? "attention"
        : "calm";

  return [
    buildWorldObject(
      {
        id: "tasks",
        sprite: "task_grove",
        label: "Task grove",
        value: `${taskTotal} open`,
        description:
          taskStuck > 0
            ? `${taskStuck} stuck branches need Buddy's nudge.`
            : "Branches are clear enough to grow.",
        page: { type: "tasks_list" },
        tone: toneFromCount(taskPressure, 1, 3),
        x: 18,
        y: 68,
        size: 16,
      },
      {
        state: taskPressure > 0 ? "attention" : "calm",
        intensity: taskPressure > 0 ? 0.32 + taskPressure / 8 : 0.22,
        animation: taskPressure > 0 ? "wobble" : "breathe",
        interactionX: 23,
        interactionY: 76,
        depthScale: 0.96,
        magicalLabel: taskPressure > 0 ? "Restless grove" : "Task grove",
      },
    ),
    buildWorldObject(
      {
        id: "memory",
        sprite: "memory_fireflies",
        label: "Memory fireflies",
        value: `${memoryTotal} docs`,
        description:
          memoryIssues > 0
            ? `${memoryIssues} memory sparks want pruning.`
            : "Knowledge fireflies are neatly orbiting.",
        page: { type: "knowledge_graph" },
        tone: toneFromCount(memoryIssues, 1, 6),
        x: 33,
        y: 52,
        size: 14,
      },
      {
        state: memoryState,
        intensity: memoryRuntimeActive
          ? 0.84
          : memoryIssues > 0
            ? 0.36 + memoryLoad / 12
            : 0.28,
        animation: memoryRuntimeActive
          ? "stream"
          : memoryIssues > 0
            ? "orbit"
            : "sparkle",
        interactionX: 36,
        interactionY: 72,
        depthScale: 0.9,
        magicalLabel:
          memoryIssues > 0 ? "Whispering fireflies" : "Memory fireflies",
      },
    ),
    buildWorldObject(
      {
        id: "providers",
        sprite: "observatory",
        label: "Model observatory",
        value: pulse.providers.defaults_ok ? "Defaults ok" : "Defaults off",
        description:
          providerIssues > 0
            ? `${providerIssues} provider signals are flickering.`
            : "Model stars are aligned.",
        page: { type: "default_models" },
        tone: toneFromCount(providerIssues, 1, 3),
        x: 72,
        y: 67,
        size: 18,
      },
      {
        state: providerState,
        intensity: providerCritical
          ? 1
          : providerRuntimeActive
            ? 0.82
            : providerWarnings > 0
              ? 0.56
              : 0.28,
        animation: providerCritical
          ? "storm"
          : providerRuntimeActive
            ? "stream"
            : providerWarnings > 0
              ? "flicker"
              : "sparkle",
        interactionX: 67,
        interactionY: 74,
        depthScale: 1.02,
        magicalLabel: providerCritical
          ? "Crackling observatory"
          : providerWarnings > 0
            ? "Flickering observatory"
            : "Model observatory",
      },
    ),
    buildWorldObject(
      {
        id: "mcp",
        sprite: "satellite",
        label: "MCP satellites",
        value: `${safeCount(pulse.mcp.total)} linked`,
        description:
          mcpFailing > 0 || mcpAuthExpiring > 0
            ? `${mcpFailing} failing · ${mcpAuthExpiring} auth expiring.`
            : "Satellites are holding orbit.",
        page: { type: "integrations" },
        tone: toneFromCount(mcpPressure, 1, 3),
        x: 84,
        y: 35,
        size: 13,
      },
      {
        state:
          mcpPressure >= 3
            ? "critical"
            : mcpPressure > 0
              ? "attention"
              : "calm",
        intensity: mcpPressure > 0 ? 0.32 + mcpPressure / 8 : 0.22,
        animation: mcpPressure > 0 ? "flicker" : "orbit",
        interactionX: 78,
        interactionY: 72,
        depthScale: 0.84,
        magicalLabel:
          mcpPressure > 0 ? "Wavering satellites" : "MCP satellites",
      },
    ),
    buildWorldObject(
      {
        id: "git",
        sprite: "git_vane",
        label: "Git weather vane",
        value: `${gitUncommittedFiles} files`,
        description:
          gitDiffLines > 0
            ? `${gitDiffLines} lines moved in the last 4h.`
            : "No diff winds right now.",
        page: { type: "stats" },
        tone: toneFromCount(gitUncommittedFiles, 8, 20),
        x: 29,
        y: 78,
        size: 12,
      },
      {
        state: gitPressure > 0 ? "attention" : "calm",
        intensity: gitPressure > 0 ? 0.3 + gitUncommittedFiles / 40 : 0.18,
        animation: gitPressure > 0 ? "wobble" : "breathe",
        interactionX: 30,
        interactionY: 80,
        depthScale: 1.06,
        magicalLabel: gitPressure > 0 ? "Rustling vane" : "Git weather vane",
      },
    ),
    buildWorldObject(
      {
        id: "market",
        sprite: "market_comet",
        label: "Upgrade comet",
        value: `${customizationTools} tools`,
        description: `${safeCount(
          pulse.customization.modes,
        )} modes · ${safeCount(
          pulse.customization.subagents,
        )} delegates · ${safeCount(pulse.customization.hooks)} hooks.`,
        page: { type: "marketplace_hub" },
        tone: "neutral",
        x: 36,
        y: 38,
        size: 13,
      },
      {
        state: "calm",
        intensity: 0.26,
        animation: "sparkle",
        interactionX: 43,
        interactionY: 70,
        depthScale: 0.82,
        magicalLabel: "Upgrade comet",
      },
    ),
  ];
}

function weatherFromState(
  phase: BuddyWorldPhase,
  pulse: BuddyPulse | null | undefined,
  pet: BuddyPetState | undefined,
  visibleRuntime: BuddyRuntimeEvent | null,
): Pick<
  BuddyWorldState,
  "weather" | "weatherLabel" | "weatherDescription" | "weatherX" | "weatherY"
> {
  if (pet?.condition.sleeping) {
    return {
      weather: "dream",
      weatherLabel: "Dream mist",
      weatherDescription: "Buddy is asleep; the world lowers its volume.",
      weatherX: 61,
      weatherY: 30,
    };
  }

  if (isProviderModelRuntimeProblem(visibleRuntime)) {
    return {
      weather: "storm",
      weatherLabel: "Bug storm",
      weatherDescription:
        visibleRuntime?.title ??
        "Errors are crackling; Buddy can chase them down.",
      weatherX: 57,
      weatherY: 27,
    };
  }

  if (visibleRuntime !== null && isActiveRuntime(visibleRuntime)) {
    return {
      weather: "busy",
      weatherLabel: "Busy currents",
      weatherDescription: visibleRuntime.title,
      weatherX: 50,
      weatherY: 61,
    };
  }

  if (pulse) {
    if (hasProviderModelPulseProblem(pulse)) {
      return {
        weather: "storm",
        weatherLabel: "Bug storm",
        weatherDescription: "Errors are crackling; Buddy can chase them down.",
        weatherX: 57,
        weatherY: 27,
      };
    }

    if (memoryIssueCount(pulse) >= 3) {
      return {
        weather: "rain",
        weatherLabel: "Memory rain",
        weatherDescription: "Old notes are watering new cleanup work.",
        weatherX: 42,
        weatherY: 28,
      };
    }

    if (
      safeCount(pulse.git.diff_lines_4h) > 0 ||
      safeCount(pulse.git.uncommitted_files) > 0
    ) {
      return {
        weather: "wind",
        weatherLabel: "Diff breeze",
        weatherDescription: "Recent changes are rustling through the garden.",
        weatherX: 44,
        weatherY: 25,
      };
    }
  }

  if (phase === "night") {
    return {
      weather: "aurora",
      weatherLabel: "Quiet aurora",
      weatherDescription: "Night signals are calm enough to sparkle.",
      weatherX: 42,
      weatherY: 24,
    };
  }

  return {
    weather: "clear",
    weatherLabel: "Clear sky",
    weatherDescription: "Buddy has room to explore and play.",
    weatherX: 42,
    weatherY: 24,
  };
}

function vitalityFromPulse(
  pulse: BuddyPulse | null | undefined,
): Pick<BuddyWorldState, "vitality" | "vitalityLabel"> {
  if (!pulse) {
    return { vitality: "growing", vitalityLabel: "Sprouting" };
  }

  const attention =
    safeCount(pulse.tasks.stuck) * 10 +
    safeCount(pulse.tasks.abandoned) * 8 +
    diagnosticPressure(pulse) * 4 +
    providerCriticalCount(pulse) * 12 +
    safeCount(pulse.mcp.failing) * 8 +
    safeCount(pulse.memory.stale_conflicts) * 6 +
    Math.min(24, safeCount(pulse.git.uncommitted_files));

  if (attention >= 60) return { vitality: "tangled", vitalityLabel: "Tangled" };
  if (attention >= 20) return { vitality: "growing", vitalityLabel: "Growing" };
  return { vitality: "lush", vitalityLabel: "Lush" };
}

function phasePaletteHint(
  phase: BuddyWorldPhase,
): BuddyWorldAtmosphere["paletteHint"] {
  switch (phase) {
    case "morning":
      return "dawn";
    case "day":
      return "day";
    case "evening":
      return "dusk";
    case "night":
      return "night";
  }
}

function hasAffectionState(args: {
  pet: BuddyPetState | undefined;
  semanticState: BuddySemanticState | undefined;
  nowMs: number;
}): boolean {
  const affection = args.pet?.needs.affection;
  if (
    typeof affection === "number" &&
    Number.isFinite(affection) &&
    affection >= 70
  ) {
    return true;
  }
  const lastSignalType = args.semanticState?.activity.lastSignalType;
  const lastSignalTime = args.semanticState?.activity.lastSignalTime;
  if (
    lastSignalType == null ||
    !AFFECTION_SIGNALS.has(lastSignalType) ||
    typeof lastSignalTime !== "number" ||
    !Number.isFinite(lastSignalTime) ||
    !Number.isFinite(args.nowMs)
  ) {
    return false;
  }
  return args.nowMs >= lastSignalTime && args.nowMs - lastSignalTime <= 600_000;
}

function buildAtmosphere(args: {
  phase: BuddyWorldPhase;
  primaryWeather: BuddyWorldWeather;
  pulse: BuddyPulse | null | undefined;
  pet: BuddyPetState | undefined;
  visibleRuntime: BuddyRuntimeEvent | null;
  semanticState: BuddySemanticState | undefined;
  nowMs: number;
}): BuddyWorldAtmosphere {
  const layers: BuddyWorldLayer[] = [];
  const runtimeActive = isActiveRuntime(args.visibleRuntime);
  const memoryRuntimeActive = isMemoryRuntimeActive(args.visibleRuntime);
  const providerRuntimeActive = isProviderRuntimeActive(args.visibleRuntime);
  const serious =
    hasProviderModelPulseProblem(args.pulse) ||
    isProviderModelRuntimeProblem(args.visibleRuntime);
  const sleeping = args.pet?.condition.sleeping === true;
  const hungry = args.pet?.condition.hungry === true;
  const bored = args.pet?.condition.bored === true;
  const affectionate = hasAffectionState({
    pet: args.pet,
    semanticState: args.semanticState,
    nowMs: args.nowMs,
  });
  const providerWarnings = providerWarningCount(args.pulse);
  const memoryLoad = memoryPressure(args.pulse);
  const memoryIssues = memoryIssueCount(args.pulse);
  const genericDiagnosticPressure = hasGenericDiagnosticPressure(args.pulse);

  switch (args.phase) {
    case "morning":
    case "day":
      addLayer(layers, "sun_motes");
      break;
    case "evening":
      addLayer(layers, "moths");
      addLayer(layers, "cozy_home_glow");
      break;
    case "night":
      addLayer(layers, "stars");
      addLayer(layers, "fireflies");
      break;
  }

  if (!args.pulse || affectionate) addLayer(layers, "cozy_home_glow");
  if (sleeping || args.primaryWeather === "dream")
    addLayer(layers, "dream_mist");
  if (hungry) addLayer(layers, "empty_food_nook");
  if (bored) addLayer(layers, "toy_glow");
  if (
    runtimeActive ||
    args.primaryWeather === "busy" ||
    genericDiagnosticPressure
  )
    addLayer(layers, "workshop_runes");
  if (providerWarnings > 0) addLayer(layers, "provider_flicker");
  if (serious) addLayer(layers, "provider_storm");
  if (memoryIssues > 0 || memoryRuntimeActive) addLayer(layers, "memory_orbs");
  if (providerRuntimeActive) addLayer(layers, "workshop_runes");
  if (args.primaryWeather === "aurora") addLayer(layers, "aurora");

  let mood: BuddyWorldMood = args.phase === "night" ? "serene" : "curious";
  if (affectionate) mood = "affectionate";
  if (bored) mood = "bored";
  if (hungry) mood = "hungry";
  if (runtimeActive) mood = "busy";
  if (sleeping) mood = "sleepy";
  if (serious) mood = "unstable";

  let intensity = args.phase === "night" ? 0.44 : 0.38;
  if (!args.pulse) intensity = 0.3;
  if (affectionate) intensity = Math.max(intensity, 0.42);
  if (hungry || bored) intensity = Math.max(intensity, 0.46);
  if (memoryIssues > 0) intensity = Math.max(intensity, 0.38 + memoryLoad / 20);
  if (providerWarnings > 0) intensity = Math.max(intensity, 0.52);
  if (runtimeActive) intensity = Math.max(intensity, 0.72);
  if (serious) intensity = Math.max(intensity, 0.92);
  if (sleeping && !serious) intensity = Math.min(intensity, 0.32);

  return {
    phase: args.phase,
    mood,
    primaryWeather: args.primaryWeather,
    layers,
    intensity: clamp01(intensity),
    paletteHint: serious
      ? "storm"
      : sleeping
        ? "dream"
        : phasePaletteHint(args.phase),
    serious,
  };
}

export function buildBuddyWorldState(args: {
  now: Date;
  pulse: BuddyPulse | null | undefined;
  pet: BuddyPetState | undefined;
  nowPlaying: BuddyRuntimeEvent | null;
  activeQuest: BuddyQuest | null;
  semanticState?: BuddySemanticState;
}): BuddyWorldState {
  const phase = phaseFromHour(args.now.getHours());
  const phaseInfo = phaseDetails(phase);
  const nowMs = args.now.getTime();
  const visibleRuntime = visibleRuntimeEvent(args.nowPlaying, nowMs);
  const weatherInfo = weatherFromState(
    phase,
    args.pulse,
    args.pet,
    visibleRuntime,
  );
  const vitalityInfo = vitalityFromPulse(args.pulse);
  const objects = buildObjects(args.pulse, visibleRuntime);
  const atmosphere = buildAtmosphere({
    phase,
    primaryWeather: weatherInfo.weather,
    pulse: args.pulse,
    pet: args.pet,
    visibleRuntime,
    semanticState: args.semanticState,
    nowMs,
  });
  const questText = args.activeQuest
    ? ` Quest active: ${args.activeQuest.title}.`
    : "";

  return {
    phase,
    ...phaseInfo,
    ...weatherInfo,
    ...vitalityInfo,
    objects,
    atmosphere,
    headline:
      `${phaseInfo.phaseMessage} ${weatherInfo.weatherDescription}${questText}`.trim(),
  };
}
