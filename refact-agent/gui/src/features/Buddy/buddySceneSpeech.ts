import type {
  BuddyControl,
  BuddyOpportunity,
  BuddyRuntimeEvent,
  BuddySpeechItem,
  BuddySuggestion,
} from "./types";
import {
  opportunityActionControls,
  opportunitySpeechText,
} from "./buddyOpportunityActions";
import {
  isFreshErrorWithinGrace,
  isBuddyRuntimeEventVisible,
  isErrorRuntimeEvent,
} from "./buddyRuntimeEvents";

export type BuddySceneSpeechSource =
  | "speech"
  | "runtime"
  | "suggestion"
  | "opportunity";

export interface BuddySceneSpeech {
  id: string;
  text: string;
  controls: BuddyControl[];
  mustShow?: boolean;
  chat_id?: string;
  speech_intent?: string;
  source: BuddySceneSpeechSource;
  runtimeEventId?: string;
  suggestionId?: string;
  opportunityId?: string;
}

const SPEECH_SILENCE_CHANCE = 0.35;
const LIVE_CHAT_REACTION_SIGNALS = new Set([
  "speech_humor",
  "speech_insight",
  "chat_bug_candidate",
  "speech_chat_reaction",
  "chat_interaction",
  "chat_interaction_comment",
  "interaction_comment",
  "live_interaction_reaction",
]);

function normalizeRuntimeText(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

function stripNoisyRuntimePrefixes(text: string): string {
  return normalizeRuntimeText(text)
    .replace(/\bgeneric:\s*/gi, "")
    .replace(/(?:\bLLM error:\s*){2,}/gi, "LLM error: ")
    .replace(/\bLLM error:\s*LLM error:\s*/gi, "LLM error: ")
    .trim();
}

function isContextWindowError(text: string): boolean {
  return /context window|exceeds?\s+(?:the\s+)?context|input exceeds/i.test(
    text,
  );
}

export function formatBuddyRuntimeEventText(event: BuddyRuntimeEvent): string {
  const speechText = event.speech_text?.trim();
  if (speechText) return normalizeRuntimeText(speechText);

  const description = event.description?.trim();
  const failureSummary = event.failure_summary?.trim();
  const fallbackText = description
    ? `${event.title}: ${description}`
    : event.title;
  const rawText = stripNoisyRuntimePrefixes(
    failureSummary?.length ? failureSummary : fallbackText,
  );

  if (
    isContextWindowError(rawText) ||
    event.failure_category === "context_too_large"
  ) {
    return "I ran out of context room. Want me to compress this and try again?";
  }

  if (event.status === "failed" && /\bLLM error\b/i.test(rawText)) {
    return rawText.replace(/^LLM error:\s*/i, "I hit an LLM snag: ");
  }

  return rawText;
}

function defaultRuntimeControls(event: BuddyRuntimeEvent): BuddyControl[] {
  if (!isErrorRuntimeEvent(event)) return [];

  return [
    {
      id: `investigate-${event.id}`,
      label: event.source === "frontend" ? "Bag bug" : "Sniff logs",
      action: "investigate_error",
      style: "primary",
    },
    {
      id: `dismiss-${event.id}`,
      label: "Shoo",
      action: "dismiss_runtime_event",
      action_param: event.id,
      style: "secondary",
    },
  ];
}

function runtimeEventToSpeech(
  event: BuddyRuntimeEvent | null | undefined,
): BuddySceneSpeech | null {
  if (event === null || event === undefined) return null;
  if (!isBuddyRuntimeEventVisible(event)) return null;
  const text = formatBuddyRuntimeEventText(event).trim();
  if (!text) return null;
  const controls = event.controls?.length
    ? event.controls
    : defaultRuntimeControls(event);
  const mustShow =
    isErrorRuntimeEvent(event) &&
    controls.some((control) => control.action === "dismiss_runtime_event");
  return {
    id: `runtime-${event.id}`,
    text,
    controls,
    mustShow,
    chat_id: event.chat_id,
    source: "runtime",
    runtimeEventId: event.id,
  };
}

export function isBuddySpeechExpired(
  speech: BuddySpeechItem,
  nowMs = Date.now(),
): boolean {
  if (speech.persistent) return false;
  if (speech.ttl_seconds <= 0) return false;
  const createdAt = Date.parse(speech.created_at);
  if (!Number.isFinite(createdAt)) return false;
  if (!Number.isFinite(nowMs)) return false;
  return nowMs - createdAt > speech.ttl_seconds * 1000;
}

function suggestionToSpeech(
  suggestion: BuddySuggestion | null | undefined,
): BuddySceneSpeech | null {
  if (!suggestion || suggestion.dismissed) return null;
  return {
    id: `suggestion-${suggestion.id}`,
    text: `${suggestion.title}: ${suggestion.description}`,
    controls: suggestion.controls.map((control) =>
      control.action === "dismiss"
        ? {
            ...control,
            action: "dismiss_suggestion",
            action_param: suggestion.id,
          }
        : control,
    ),
    source: "suggestion",
    suggestionId: suggestion.id,
  };
}

function opportunityToSpeech(
  opportunity: BuddyOpportunity | null | undefined,
): BuddySceneSpeech | null {
  if (
    !opportunity ||
    (opportunity.status !== "new" && opportunity.status !== "shown")
  ) {
    return null;
  }

  return {
    id: `opportunity-${opportunity.id}`,
    text: opportunitySpeechText(opportunity),
    controls: opportunityActionControls(opportunity),
    source: "opportunity",
    opportunityId: opportunity.id,
  };
}

function runtimeCandidatesFromQueue(
  nowPlaying: BuddyRuntimeEvent | null,
  runtimeQueue: BuddyRuntimeEvent[],
): BuddyRuntimeEvent[] {
  const candidates = [nowPlaying, ...runtimeQueue].filter(
    (event): event is BuddyRuntimeEvent =>
      isBuddyRuntimeEventVisible(event) &&
      formatBuddyRuntimeEventText(event).trim() !== "",
  );

  return candidates.sort(compareBuddyRuntimeEvents).slice(0, 4);
}

function normalizedRuntimeToken(value: string | null | undefined): string {
  return (
    value
      ?.trim()
      .toLowerCase()
      .replace(/[:\s-]+/g, "_") ?? ""
  );
}

function isLiveChatReactionEvent(event: BuddyRuntimeEvent): boolean {
  return (
    event.source === "chat_reactions" ||
    LIVE_CHAT_REACTION_SIGNALS.has(normalizedRuntimeToken(event.signal_type)) ||
    LIVE_CHAT_REACTION_SIGNALS.has(normalizedRuntimeToken(event.source)) ||
    LIVE_CHAT_REACTION_SIGNALS.has(
      normalizedRuntimeToken(event.dedupe_key ?? undefined),
    )
  );
}

function isFreshRuntimeEvent(event: BuddyRuntimeEvent, nowMs = Date.now()) {
  const createdAtMs = runtimeCreatedAtMs(event);
  if (createdAtMs <= 0 || !Number.isFinite(nowMs)) return false;
  if (createdAtMs > nowMs + 30_000) return false;
  const freshnessMs =
    event.ttl_ms != null && Number.isFinite(event.ttl_ms) && event.ttl_ms > 0
      ? event.ttl_ms
      : 75_000;
  return nowMs - createdAtMs <= freshnessMs;
}

function isPersistentActiveProgressEvent(event: BuddyRuntimeEvent): boolean {
  if (isErrorRuntimeEvent(event)) return false;
  if (event.persistent !== true) return false;
  return (
    event.status === "started" ||
    event.status === "progress" ||
    event.status === "streaming"
  );
}

function runtimePriorityScore(event: BuddyRuntimeEvent): number {
  if (event.priority === "critical" && isFreshErrorWithinGrace(event)) {
    return 900;
  }
  if (isPersistentActiveProgressEvent(event) && isFreshRuntimeEvent(event)) {
    return 875;
  }
  if (event.priority === "high" && isFreshErrorWithinGrace(event)) return 860;
  if (isLiveChatReactionEvent(event) && isFreshRuntimeEvent(event)) return 850;
  if (isErrorRuntimeEvent(event)) return 50;

  const priorityScore = (() => {
    switch (event.priority) {
      case "critical":
        return 400;
      case "high":
        return 300;
      case "normal":
        return 100;
      case "low":
        return 0;
      default:
        return 50;
    }
  })();

  const statusScore = (() => {
    switch (event.status) {
      case "failed":
        return 500;
      case "started":
      case "progress":
      case "streaming":
        return 300;
      case "info":
        return 150;
      case "completed":
        return 25;
      default:
        return isErrorRuntimeEvent(event) ? 500 : 0;
    }
  })();

  const hasControlsScore = event.controls?.length ? 20 : 0;
  return priorityScore + statusScore + hasControlsScore;
}

function runtimeCreatedAtMs(event: BuddyRuntimeEvent): number {
  const timestamp = Date.parse(event.created_at);
  return Number.isFinite(timestamp) ? timestamp : 0;
}

export function compareBuddyRuntimeEvents(
  left: BuddyRuntimeEvent,
  right: BuddyRuntimeEvent,
): number {
  const scoreDiff = runtimePriorityScore(right) - runtimePriorityScore(left);
  if (scoreDiff !== 0) return scoreDiff;
  return runtimeCreatedAtMs(right) - runtimeCreatedAtMs(left);
}

function stableHash(text: string): number {
  let hash = 0;
  for (let index = 0; index < text.length; index += 1) {
    hash = (hash * 31 + text.charCodeAt(index)) >>> 0;
  }
  return hash;
}

function bucketedRandom(seed: string, bucketMs: number): number {
  const bucket = Math.floor(Date.now() / bucketMs);
  return (stableHash(`${seed}:${bucket}`) % 10_000) / 10_000;
}

function shuffleCandidates(candidates: BuddySceneSpeech[]): BuddySceneSpeech[] {
  const bucket = Math.floor(Date.now() / 45_000);
  return [...candidates]
    .map((candidate, index) => ({
      candidate,
      score: stableHash(`${candidate.id}:${bucket}`),
      index,
    }))
    .sort((left, right) => left.score - right.score || left.index - right.index)
    .map(({ candidate }) => candidate);
}

export function buildBuddySceneSpeech(args: {
  activeSpeech: BuddySpeechItem | null;
  nowPlaying: BuddyRuntimeEvent | null;
  runtimeQueue: BuddyRuntimeEvent[];
  activeSuggestion?: BuddySuggestion | null;
  activeOpportunities?: BuddyOpportunity[];
}): BuddySceneSpeech | null {
  if (args.activeSpeech && !isBuddySpeechExpired(args.activeSpeech)) {
    return {
      id: `speech-${args.activeSpeech.id}`,
      text: args.activeSpeech.text,
      controls: args.activeSpeech.controls,
      chat_id: args.activeSpeech.chat_id,
      speech_intent: args.activeSpeech.speech_intent,
      source: "speech",
    };
  }

  return buildBuddySceneSpeechCandidates(args)[0] ?? null;
}

function isMustShowRuntimeSpeech(candidate: BuddySceneSpeech): boolean {
  return candidate.source === "runtime" && candidate.mustShow === true;
}

export function pickBuddySceneSpeechCandidate(
  candidates: BuddySceneSpeech[],
): BuddySceneSpeech | null {
  const mustShowCandidates = candidates.filter(isMustShowRuntimeSpeech);
  if (mustShowCandidates.length > 0) {
    return shuffleCandidates(mustShowCandidates)[0] ?? null;
  }
  if (bucketedRandom("buddy-scene-silence", 30_000) < SPEECH_SILENCE_CHANCE) {
    return null;
  }
  return shuffleCandidates(candidates)[0] ?? null;
}

export function buildBuddySceneSpeechCandidates(args: {
  nowPlaying: BuddyRuntimeEvent | null;
  runtimeQueue: BuddyRuntimeEvent[];
  activeSuggestion?: BuddySuggestion | null;
  activeOpportunities?: BuddyOpportunity[];
}): BuddySceneSpeech[] {
  const runtimeCandidates = runtimeCandidatesFromQueue(
    args.nowPlaying,
    args.runtimeQueue,
  )
    .map(runtimeEventToSpeech)
    .filter((speech): speech is BuddySceneSpeech => speech !== null);

  const opportunityCandidates = (args.activeOpportunities ?? [])
    .map(opportunityToSpeech)
    .filter((speech): speech is BuddySceneSpeech => speech !== null)
    .slice(0, 3);

  const suggestionCandidate = suggestionToSpeech(args.activeSuggestion);

  return [
    ...runtimeCandidates,
    ...opportunityCandidates,
    ...(suggestionCandidate ? [suggestionCandidate] : []),
  ];
}
