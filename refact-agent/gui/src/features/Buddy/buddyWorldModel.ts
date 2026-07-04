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
export type BuddyWorldSeason = "spring" | "summer" | "autumn" | "winter";
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
  | "empty_food_nook"
  | "season_petals"
  | "season_leaves"
  | "season_snow"
  | "summer_shimmer"
  | "morning_fog"
  | "birds"
  | "butterflies"
  | "owl"
  | "shooting_stars"
  | "rainbow"
  | "pond_life"
  | "lanterns"
  | "campfire"
  | "quest_mailbox";

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

function identityName(semanticState: BuddySemanticState | undefined): string {
  return semanticState?.name.trim() ?? "Your companion";
}
export type BuddyWorldSprite =
  | "task_grove"
  | "memory_fireflies"
  | "observatory"
  | "satellite"
  | "git_vane"
  | "market_comet"
  | "stats_totem"
  | "gear_mill"
  | "proposal_mailbox"
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
  season: BuddyWorldSeason;
  seasonLabel: string;
  moonPhase: number;
  dayProgress: number;
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

const CRITICAL_RUNTIME_PRIORITY_VALUES = [
  "critical",
  "urgent",
  "fatal",
  "danger",
] as const;

const CRITICAL_RUNTIME_SIGNALS = new Set([
  "provider_failed",
  "provider_failure",
  "provider_error",
  "model_failed",
  "model_failure",
  "model_error",
  "generation_failed",
  "generation_failure",
  "stream_failed",
  "stream_failure",
  "default_model_missing",
  "model_not_found",
  "broken_model_reference",
]);

type RuntimeEventExpiryHints = BuddyRuntimeEvent & {
  expires?: string | number | null;
  expires_at?: string | number | null;
  expiresAt?: string | number | null;
  expires_ms?: number | null;
  expiresMs?: number | null;
};

const PROVIDER_MODEL_STRICT_TOPIC_PATTERNS = [
  /\bproviders?\b/u,
  /\bprovider[-_\s]?sources?\b/u,
  /\bmodel[-_\s]?providers?\b/u,
  /\bdefault[-_\s]?models?\b/u,
  /\bdefault_model(?:_missing)?\b/u,
  /\bmodel_not_found\b/u,
  /\bbroken[-_\s]?model[-_\s]?references?\b/u,
  /\bbroken_model_reference\b/u,
  /\bllm\b/u,
  /\bcontext[-_\s]?windows?\b/u,
  /\bopenai\b/u,
  /\banthropic\b/u,
  /\bclaude\b/u,
  /\bgemini\b/u,
  /\bgroq\b/u,
  /\bollama\b/u,
  /\bopenrouter\b/u,
  /\bvllm\b/u,
  /\bxai\b/u,
] as const;

const PROVIDER_MODEL_CONTEXT_PATTERNS = [
  /\bproviders?\b/u,
  /\bprovider[-_\s]?sources?\b/u,
  /\bmodel[-_\s]?providers?\b/u,
  /\bdefault[-_\s]?models?\b/u,
  /\bdefault_model\b/u,
] as const;

const PROVIDER_MODEL_CONTEXTUAL_TOPIC_PATTERNS = [
  /\bapi[-_\s]?keys?\b/u,
  /\bapikeys?\b/u,
  /\brate[-_\s]?limits?\b/u,
  /\bquotas?\b/u,
  /\bmodels?\b/u,
  /\bbroken[-_\s]?refs?\b/u,
  /\bbroken_refs?\b/u,
] as const;

const AFFECTION_SIGNAL_WINDOW_MS = 600_000;
const AFFECTION_SIGNAL_FUTURE_TOLERANCE_MS = 5_000;

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

function safeStrings(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((item): item is string => typeof item === "string");
}

function petConditionFlag(
  pet: BuddyPetState | undefined,
  key: keyof BuddyPetState["condition"],
): boolean {
  const condition: Partial<BuddyPetState["condition"]> | undefined =
    pet?.condition;
  return condition?.[key] === true;
}

function petNeedValue(
  pet: BuddyPetState | undefined,
  key: keyof BuddyPetState["needs"],
): number | undefined {
  const needs: Partial<BuddyPetState["needs"]> | undefined = pet?.needs;
  const value = needs?.[key];
  return typeof value === "number" ? value : undefined;
}

function normalizeBuddyPulse(
  pulse: BuddyPulse | null | undefined,
): BuddyPulse | null {
  if (!pulse) return null;
  const raw: Partial<BuddyPulse> = pulse;
  return {
    generated_at: raw.generated_at ?? null,
    tasks: {
      total: safeCount(raw.tasks?.total),
      stuck: safeCount(raw.tasks?.stuck),
      abandoned: safeCount(raw.tasks?.abandoned),
      by_status: raw.tasks?.by_status ?? {},
    },
    trajectories: {
      total: safeCount(raw.trajectories?.total),
      untitled: safeCount(raw.trajectories?.untitled),
      oldest_age_days: safeCount(raw.trajectories?.oldest_age_days),
    },
    memory: {
      total: safeCount(raw.memory?.total),
      orphan: safeCount(raw.memory?.orphan),
      stale_conflicts: safeCount(raw.memory?.stale_conflicts),
    },
    providers: {
      defaults_ok: raw.providers?.defaults_ok !== false,
      broken_refs: safeCount(raw.providers?.broken_refs),
      quota_warnings: safeCount(raw.providers?.quota_warnings),
    },
    mcp: {
      total: safeCount(raw.mcp?.total),
      failing: safeCount(raw.mcp?.failing),
      auth_expiring: safeCount(raw.mcp?.auth_expiring),
    },
    customization: {
      modes: safeCount(raw.customization?.modes),
      skills: safeCount(raw.customization?.skills),
      commands: safeCount(raw.customization?.commands),
      subagents: safeCount(raw.customization?.subagents),
      hooks: safeCount(raw.customization?.hooks),
    },
    diagnostics: {
      last_hour: safeCount(raw.diagnostics?.last_hour),
      top_error_types: safeStrings(raw.diagnostics?.top_error_types),
    },
    git: {
      uncommitted_files: safeCount(raw.git?.uncommitted_files),
      diff_lines_4h: safeCount(raw.git?.diff_lines_4h),
      branches: safeCount(raw.git?.branches),
    },
    worktrees: {
      total_registered: safeCount(raw.worktrees?.total_registered),
      total_discovered: safeCount(raw.worktrees?.total_discovered),
      total: safeCount(raw.worktrees?.total),
      clean: safeCount(raw.worktrees?.clean),
      dirty: safeCount(raw.worktrees?.dirty),
      unknown: safeCount(raw.worktrees?.unknown),
      stale: safeCount(raw.worktrees?.stale),
      conflicted: safeCount(raw.worktrees?.conflicted),
      shared: safeCount(raw.worktrees?.shared),
      abandoned_clean: safeCount(raw.worktrees?.abandoned_clean),
      changed_files: safeCount(raw.worktrees?.changed_files),
      additions: safeCount(raw.worktrees?.additions),
      deletions: safeCount(raw.worktrees?.deletions),
      missing_registry_paths: safeCount(raw.worktrees?.missing_registry_paths),
      unregistered_cache_dirs: safeCount(
        raw.worktrees?.unregistered_cache_dirs,
      ),
      merged_branches: safeCount(raw.worktrees?.merged_branches),
      newest_age_hours:
        raw.worktrees?.newest_age_hours == null
          ? null
          : safeCount(raw.worktrees.newest_age_hours),
      oldest_age_hours:
        raw.worktrees?.oldest_age_hours == null
          ? null
          : safeCount(raw.worktrees.oldest_age_hours),
      disk_usage_bytes:
        raw.worktrees?.disk_usage_bytes == null
          ? null
          : safeCount(raw.worktrees.disk_usage_bytes),
    },
    humor: raw.humor ?? null,
  };
}

function addLayer(layers: BuddyWorldLayer[], layer: BuddyWorldLayer): void {
  if (!layers.includes(layer)) layers.push(layer);
}

function runtimeEventExpiryMs(event: BuddyRuntimeEvent): number | null {
  const eventWithHints: RuntimeEventExpiryHints = event;
  const explicitExpiry =
    eventWithHints.expires ??
    eventWithHints.expires_at ??
    eventWithHints.expiresAt ??
    null;
  if (typeof explicitExpiry === "number" && Number.isFinite(explicitExpiry)) {
    return explicitExpiry;
  }
  if (typeof explicitExpiry === "string") {
    const parsedExpiry = Date.parse(explicitExpiry);
    if (Number.isFinite(parsedExpiry)) return parsedExpiry;
  }

  const explicitExpiryMs =
    eventWithHints.expires_ms ?? eventWithHints.expiresMs ?? null;
  if (
    typeof explicitExpiryMs === "number" &&
    Number.isFinite(explicitExpiryMs)
  ) {
    return explicitExpiryMs;
  }

  if (event.ttl_ms == null || !Number.isFinite(event.ttl_ms)) return null;
  const createdAtMs = Date.parse(event.created_at);
  if (!Number.isFinite(createdAtMs)) return null;
  return createdAtMs + event.ttl_ms;
}

function isRuntimeEventExpired(
  event: BuddyRuntimeEvent,
  nowMs: number,
): boolean {
  if (!Number.isFinite(nowMs)) return true;
  const expiryMs = runtimeEventExpiryMs(event);
  return expiryMs !== null && nowMs > expiryMs;
}

function hasCriticalRuntimePriority(event: BuddyRuntimeEvent): boolean {
  const priority = event.priority.toLowerCase();
  return CRITICAL_RUNTIME_PRIORITY_VALUES.some(
    (criticalPriority) =>
      priority === criticalPriority ||
      priority.startsWith(`${criticalPriority}:`) ||
      priority.startsWith(`${criticalPriority}_`) ||
      priority.startsWith(`${criticalPriority}-`),
  );
}

function hasCriticalRuntimeSignal(event: BuddyRuntimeEvent): boolean {
  const signalType = event.signal_type.toLowerCase();
  return (
    CRITICAL_RUNTIME_SIGNALS.has(signalType) ||
    signalType.endsWith("_failed") ||
    signalType.endsWith("_failure") ||
    signalType.endsWith("_error")
  );
}

function visibleRuntimeEvent(
  event: BuddyRuntimeEvent | null,
  nowMs: number,
): BuddyRuntimeEvent | null {
  if (event === null || event.dismissed === true) return null;
  if (event.persistent !== true && isRuntimeEventExpired(event, nowMs))
    return null;
  if (isActiveRuntime(event) && event.ttl_ms == null) return event;
  if (isBuddyRuntimeEventVisible(event, nowMs)) return event;
  return isProviderModelRuntimeProblem(event) ? event : null;
}

function isActiveRuntime(event: BuddyRuntimeEvent | null): boolean {
  return event !== null && ACTIVE_RUNTIME_STATUSES.has(event.status);
}

function runtimeText(event: BuddyRuntimeEvent): string {
  return `${event.signal_type} ${event.title} ${event.description ?? ""} ${
    event.source
  }`.toLowerCase();
}

function runtimeContextText(event: BuddyRuntimeEvent): string {
  return `${event.signal_type} ${event.source}`.toLowerCase();
}

function hasProviderModelTopicText(text: string): boolean {
  const normalized = text.toLowerCase();
  if (
    PROVIDER_MODEL_STRICT_TOPIC_PATTERNS.some((pattern) =>
      pattern.test(normalized),
    )
  ) {
    return true;
  }

  return (
    PROVIDER_MODEL_CONTEXT_PATTERNS.some((pattern) =>
      pattern.test(normalized),
    ) &&
    PROVIDER_MODEL_CONTEXTUAL_TOPIC_PATTERNS.some((pattern) =>
      pattern.test(normalized),
    )
  );
}

function hasProviderModelRuntimeTopic(event: BuddyRuntimeEvent): boolean {
  const text = runtimeText(event);
  if (
    PROVIDER_MODEL_STRICT_TOPIC_PATTERNS.some((pattern) => pattern.test(text))
  ) {
    return true;
  }

  return (
    PROVIDER_MODEL_CONTEXT_PATTERNS.some((pattern) =>
      pattern.test(runtimeContextText(event)),
    ) &&
    PROVIDER_MODEL_CONTEXTUAL_TOPIC_PATTERNS.some((pattern) =>
      pattern.test(text),
    )
  );
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
  if (event === null) return false;
  if (!isGenerationRuntime(event) && !hasProviderModelRuntimeTopic(event)) {
    return false;
  }
  return (
    event.status.toLowerCase() === "failed" ||
    hasCriticalRuntimePriority(event) ||
    hasCriticalRuntimeSignal(event)
  );
}

function providerWarningCount(pulse: BuddyPulse | null | undefined): number {
  if (!pulse) return 0;
  return (
    safeCount(pulse.providers.quota_warnings) +
    (pulse.providers.defaults_ok ? 0 : 1)
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
      hasProviderModelTopicText(diagnosticsTopicText(pulse)))
  );
}

function phaseFromHour(hour: number): BuddyWorldPhase {
  if (hour >= 5 && hour < 11) return "morning";
  if (hour >= 11 && hour < 17) return "day";
  if (hour >= 17 && hour < 21) return "evening";
  return "night";
}

const SEASON_LABELS: Record<BuddyWorldSeason, string> = {
  spring: "Blossom season",
  summer: "High sun season",
  autumn: "Falling leaves season",
  winter: "Quiet snow season",
};

function seasonFromMonth(month: number): BuddyWorldSeason {
  if (month >= 2 && month <= 4) return "spring";
  if (month >= 5 && month <= 7) return "summer";
  if (month >= 8 && month <= 10) return "autumn";
  return "winter";
}

const SYNODIC_MONTH_DAYS = 29.530588;
const NEW_MOON_REF_MS = Date.UTC(2024, 0, 11, 11, 57);

function moonPhaseFromTime(nowMs: number): number {
  if (!Number.isFinite(nowMs)) return 0.5;
  const days = (nowMs - NEW_MOON_REF_MS) / 86_400_000;
  const phase = (days / SYNODIC_MONTH_DAYS) % 1;
  return phase < 0 ? phase + 1 : phase;
}

function celestialArc(now: Date): {
  dayProgress: number;
  x: number;
  y: number;
} {
  const hour = now.getHours() + now.getMinutes() / 60;
  const daytime = hour >= 5 && hour < 21;
  const raw = daytime
    ? (hour - 5) / 16
    : (hour >= 21 ? hour - 21 : hour + 3) / 8;
  const progress = clampRange(raw, 0, 1, 0.5);
  return {
    dayProgress: progress,
    x: 12 + progress * 76,
    y: (daytime ? 34 : 30) - Math.sin(progress * Math.PI) * (daytime ? 22 : 16),
  };
}

function phaseDetails(
  phase: BuddyWorldPhase,
  name: string,
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
        phaseMessage: `Everything is bright enough for ${name} to inspect.`,
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
        phaseMessage: `The moon is up and ${name} is watching quiet queues.`,
        celestialEmoji: "🌙",
        celestialLabel: "Moon",
        celestialAction: `Let ${name} rest`,
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

function buildProposalMailbox(
  pendingCards: number,
  name: string,
): BuddyWorldObject {
  return buildWorldObject(
    {
      id: "proposal-mailbox",
      sprite: "proposal_mailbox",
      label: "Mailbox",
      value: `${pendingCards} waiting`,
      description: `${name} left ${pendingCards} proposal(s) in the mailbox.`,
      page: { type: "buddy" },
      tone: pendingCards > 0 ? "warning" : "neutral",
      x: 12,
      y: 78,
      size: 10,
    },
    {
      state: pendingCards > 0 ? "attention" : "calm",
      intensity: pendingCards > 0 ? 0.7 : 0.3,
      animation: pendingCards > 0 ? "wobble" : "breathe",
      interactionX: 17,
      interactionY: 82,
      depthScale: 0.95,
      magicalLabel: "Proposal mailbox",
    },
  );
}

function buildObjects(
  pulse: BuddyPulse | null | undefined,
  visibleRuntime: BuddyRuntimeEvent | null,
  name: string,
  pendingCards: number,
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
          description: `${name} is waiting for a pulse snapshot.`,
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
      ...(pendingCards > 0 ? [buildProposalMailbox(pendingCards, name)] : []),
    ];
  }

  const providerWarnings = providerWarningCount(pulse);
  const providerPulseProblem = hasProviderModelPulseProblem(pulse);
  const providerCritical = providerPulseProblem || runtimeProviderProblem;
  const providerIssues =
    providerCriticalCount(pulse) +
    providerWarnings +
    (providerPulseProblem && providerCriticalCount(pulse) === 0 ? 1 : 0);
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
            ? `${taskStuck} stuck branches need ${name}'s nudge.`
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
        tone: providerCritical ? "danger" : toneFromCount(providerIssues, 1, 3),
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
        label: "Marketplace comet",
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
        magicalLabel: "Marketplace comet",
      },
    ),
    buildWorldObject(
      {
        id: "stats",
        sprite: "stats_totem",
        label: "Stats totem",
        value: `${safeCount(pulse.trajectories.total)} chats`,
        description: "Usage, costs and activity charts.",
        page: { type: "stats" },
        tone: "neutral",
        x: 90,
        y: 74,
        size: 12,
      },
      {
        state: "calm",
        intensity: 0.24,
        animation: "breathe",
        interactionX: 87,
        interactionY: 79,
        depthScale: 1.04,
        magicalLabel: "Score totem",
      },
    ),
    buildWorldObject(
      {
        id: "settings",
        sprite: "gear_mill",
        label: "Settings mill",
        value: "Preferences",
        description: `Tune ${name}'s home, looks and controls.`,
        page: { type: "settings" },
        tone: "neutral",
        x: 63,
        y: 74,
        size: 14,
      },
      {
        state: "calm",
        intensity: 0.24,
        animation: "breathe",
        interactionX: 61,
        interactionY: 78,
        depthScale: 0.94,
        magicalLabel: "Tinker mill",
      },
    ),
    ...(pendingCards > 0 ? [buildProposalMailbox(pendingCards, name)] : []),
  ];
}

function weatherFromState(
  phase: BuddyWorldPhase,
  pulse: BuddyPulse | null | undefined,
  pet: BuddyPetState | undefined,
  visibleRuntime: BuddyRuntimeEvent | null,
  name: string,
): Pick<
  BuddyWorldState,
  "weather" | "weatherLabel" | "weatherDescription" | "weatherX" | "weatherY"
> {
  if (isProviderModelRuntimeProblem(visibleRuntime)) {
    return {
      weather: "storm",
      weatherLabel: "Bug storm",
      weatherDescription:
        visibleRuntime?.title ??
        `Errors are crackling; ${name} can chase them down.`,
      weatherX: 57,
      weatherY: 27,
    };
  }

  if (
    petConditionFlag(pet, "sleeping") &&
    pulse &&
    hasProviderModelPulseProblem(pulse)
  ) {
    return {
      weather: "storm",
      weatherLabel: "Bug storm",
      weatherDescription: `Errors are crackling; ${name} can chase them down.`,
      weatherX: 57,
      weatherY: 27,
    };
  }

  if (petConditionFlag(pet, "sleeping")) {
    return {
      weather: "dream",
      weatherLabel: "Dream mist",
      weatherDescription: `${name} is asleep; the world lowers its volume.`,
      weatherX: 61,
      weatherY: 30,
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

  if (pulse && hasProviderModelPulseProblem(pulse)) {
    return {
      weather: "storm",
      weatherLabel: "Bug storm",
      weatherDescription: `Errors are crackling; ${name} can chase them down.`,
      weatherX: 57,
      weatherY: 27,
    };
  }

  if (pulse) {
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
    weatherDescription: `${name} has room to explore and play.`,
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
  const affection = petNeedValue(args.pet, "affection");
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
  return (
    args.nowMs + AFFECTION_SIGNAL_FUTURE_TOLERANCE_MS >= lastSignalTime &&
    args.nowMs - lastSignalTime <= AFFECTION_SIGNAL_WINDOW_MS
  );
}

function buildAtmosphere(args: {
  phase: BuddyWorldPhase;
  season: BuddyWorldSeason;
  hasQuest: boolean;
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
  const sleeping = petConditionFlag(args.pet, "sleeping");
  const hungry = petConditionFlag(args.pet, "hungry");
  const bored = petConditionFlag(args.pet, "bored");
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

  const calmSky =
    args.primaryWeather === "clear" ||
    args.primaryWeather === "wind" ||
    args.primaryWeather === "aurora";
  const daylight = args.phase === "morning" || args.phase === "day";
  switch (args.season) {
    case "spring":
      if (calmSky) addLayer(layers, "season_petals");
      break;
    case "summer":
      if (calmSky && daylight) addLayer(layers, "summer_shimmer");
      break;
    case "autumn":
      if (args.primaryWeather !== "storm" && args.primaryWeather !== "dream")
        addLayer(layers, "season_leaves");
      if (args.phase === "morning") addLayer(layers, "morning_fog");
      break;
    case "winter":
      if (args.primaryWeather !== "storm" && args.primaryWeather !== "dream")
        addLayer(layers, "season_snow");
      break;
  }
  if (daylight && calmSky && args.season !== "winter" && !sleeping)
    addLayer(layers, "birds");
  if (
    (args.season === "spring" || args.season === "summer") &&
    daylight &&
    args.primaryWeather === "clear" &&
    !sleeping
  )
    addLayer(layers, "butterflies");
  if (args.phase === "night" && !serious) addLayer(layers, "owl");
  if (
    args.phase === "night" &&
    (args.primaryWeather === "clear" || args.primaryWeather === "aurora")
  )
    addLayer(layers, "shooting_stars");
  if (args.primaryWeather === "rain" && args.phase === "day")
    addLayer(layers, "rainbow");
  if (args.season !== "winter") addLayer(layers, "pond_life");
  if (args.phase === "evening" || args.phase === "night")
    addLayer(layers, "lanterns");
  if (args.phase === "night" && calmSky && !sleeping)
    addLayer(layers, "campfire");
  if (args.hasQuest) addLayer(layers, "quest_mailbox");

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
  pendingCards?: number;
}): BuddyWorldState {
  const pulse = normalizeBuddyPulse(args.pulse);
  const phase = phaseFromHour(args.now.getHours());
  const season = seasonFromMonth(args.now.getMonth());
  const name = identityName(args.semanticState);
  const phaseInfo = phaseDetails(phase, name);
  const nowMs = args.now.getTime();
  const arc = celestialArc(args.now);
  const visibleRuntime = visibleRuntimeEvent(args.nowPlaying, nowMs);
  const weatherInfo = weatherFromState(
    phase,
    pulse,
    args.pet,
    visibleRuntime,
    name,
  );
  const vitalityInfo = vitalityFromPulse(pulse);
  const objects = buildObjects(
    pulse,
    visibleRuntime,
    name,
    args.pendingCards ?? 0,
  );
  const atmosphere = buildAtmosphere({
    phase,
    season,
    hasQuest: args.activeQuest != null,
    primaryWeather: weatherInfo.weather,
    pulse,
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
    season,
    seasonLabel: SEASON_LABELS[season],
    moonPhase: moonPhaseFromTime(nowMs),
    dayProgress: arc.dayProgress,
    ...phaseInfo,
    celestialX: arc.x,
    celestialY: arc.y,
    ...weatherInfo,
    ...vitalityInfo,
    objects,
    atmosphere,
    headline:
      `${phaseInfo.phaseMessage} ${weatherInfo.weatherDescription}${questText}`.trim(),
  };
}
