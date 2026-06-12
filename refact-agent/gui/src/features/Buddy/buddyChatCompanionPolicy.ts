import type { BuddyRuntimeEvent } from "./types";
import { isErrorRuntimeEvent } from "./buddyRuntimeEvents";

export const CHAT_COMPANION_STARTUP_QUIET_MS = 60_000;
export const CHAT_COMPANION_OPEN_QUIET_MS = 120_000;
export const CHAT_COMPANION_BUBBLE_GAP_MS = 90_000;

export const AMBIENT_SIGNALS = new Set<string>([
  "speech_humor",
  "speech_insight",
  "speech_chat_reaction",
  "chat_reaction",
  "speech_memory_pulse_commentary",
  "speaker_insight",
  "speaker_memory_pulse_commentary",
]);

export const AMBIENT_INTENTS = new Set<string>([
  "humor",
  "insight",
  "interaction_comment",
  "memory_pulse_commentary",
]);

export const LIVE_CHAT_REACTION_SIGNALS = new Set<string>([
  "speech_humor",
  "speech_insight",
  "chat_bug_candidate",
  "speech_chat_reaction",
  "chat_reaction",
  "chat_interaction",
  "chat_interaction_comment",
  "interaction_comment",
  "live_interaction_reaction",
]);

export const DURABLE_SPEECH_INTENTS = new Set<string>([
  "tour",
  "quest_accept",
  "quest_complete",
  "milestone",
  "win",
  "suggestion",
  "error_alert",
]);

const ACTIVE_PROGRESS_STATUS_TOKENS = new Set<string>([
  "started",
  "starting",
  "progress",
  "streaming",
  "running",
  "queued",
  "generating",
  "working",
]);

export function normalizedPolicyToken(
  value: string | null | undefined,
): string {
  const token =
    value
      ?.trim()
      .toLowerCase()
      .replace(/[:\s-]+/g, "_") ?? "";
  return token.startsWith("speech_") ? token.slice("speech_".length) : token;
}

export function isAmbientToken(value: string | null | undefined): boolean {
  const token = normalizedPolicyToken(value);
  if (!token) return false;
  return AMBIENT_INTENTS.has(token) || AMBIENT_SIGNALS.has(token);
}

export function isLiveChatReactionSignal(
  value: string | null | undefined,
): boolean {
  const token = normalizedPolicyToken(value);
  if (!token) return false;
  return LIVE_CHAT_REACTION_SIGNALS.has(token);
}

export function isLiveChatReactionEvent(event: BuddyRuntimeEvent): boolean {
  return (
    event.source === "chat_reactions" ||
    isLiveChatReactionSignal(event.signal_type) ||
    isLiveChatReactionSignal(event.source) ||
    isLiveChatReactionSignal(event.dedupe_key ?? undefined)
  );
}

export function isDurableSpeechToken(
  value: string | null | undefined,
): boolean {
  const token = normalizedPolicyToken(value);
  return token ? DURABLE_SPEECH_INTENTS.has(token) : false;
}

function isDurablePolicyEvent(event: BuddyRuntimeEvent): boolean {
  return (
    event.bubble_policy === "durable" ||
    isDurableSpeechToken(event.signal_type) ||
    isDurableSpeechToken(event.source) ||
    isDurableSpeechToken(event.dedupe_key ?? undefined)
  );
}

function isAmbientPolicyEvent(event: BuddyRuntimeEvent): boolean {
  return (
    event.bubble_policy === "ambient" ||
    isAmbientToken(event.signal_type) ||
    isAmbientToken(event.source) ||
    isAmbientToken(event.dedupe_key ?? undefined)
  );
}

export function isChatCompanionWorthyRuntimeEvent(
  event: BuddyRuntimeEvent,
): boolean {
  if (isErrorRuntimeEvent(event)) return true;
  if (isLiveChatReactionEvent(event)) return true;
  if (isAmbientPolicyEvent(event)) return true;
  if (isDurablePolicyEvent(event)) return true;
  if (ACTIVE_PROGRESS_STATUS_TOKENS.has(normalizedPolicyToken(event.status))) {
    return false;
  }
  return (event.controls?.length ?? 0) > 0;
}

const CONTENT_KEY_TEXT_LIMIT = 200;

function normalizedContentText(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/\s+/g, " ")
    .slice(0, CONTENT_KEY_TEXT_LIMIT);
}

/**
 * Stable content identity for notifications whose backend ids change between
 * re-emissions (quest prompts, repeated errors, regenerated suggestions).
 * A non-null key marks "the same message" so it is shown only once even when
 * the source object is recreated with a fresh id. Ambient chatter returns
 * null on purpose: it is throttled by the ambient gap, not by identity.
 */
export function speechContentKey(speech: {
  dedupe_key?: string;
  speech_intent?: string;
}): string | null {
  if (
    isAmbientToken(speech.speech_intent) ||
    isAmbientToken(speech.dedupe_key)
  ) {
    return null;
  }
  const token = normalizedPolicyToken(speech.dedupe_key);
  return token ? `content:speech:${token}` : null;
}

export function runtimeEventContentKey(
  event: BuddyRuntimeEvent,
  formattedText: string,
): string | null {
  if (
    isAmbientPolicyEvent(event) ||
    (isLiveChatReactionEvent(event) && !isErrorRuntimeEvent(event))
  ) {
    return null;
  }
  const token = normalizedPolicyToken(event.dedupe_key);
  if (token) return `content:runtime:${token}`;
  if (isErrorRuntimeEvent(event)) {
    const text = normalizedContentText(formattedText);
    return text ? `content:runtime:error:${text}` : null;
  }
  return null;
}

export function suggestionContentKey(suggestion: {
  suggestion_type: string;
  title: string;
}): string {
  return `content:suggestion:${normalizedPolicyToken(
    suggestion.suggestion_type,
  )}:${normalizedContentText(suggestion.title)}`;
}

export function opportunityContentKey(opportunity: {
  kind: string;
  cooldown_key?: string | null;
}): string | null {
  const key = opportunity.cooldown_key?.trim();
  if (!key) return null;
  return `content:opportunity:${normalizedContentText(
    `${opportunity.kind}:${key}`,
  )}`;
}

export type ChatCompanionGateReason =
  | "shown"
  | "startup_quiet"
  | "queue_busy"
  | "cooldown";

export interface ChatCompanionGateInput {
  nowMs: number;
  quietUntilMs: number | null;
  queuedMessageCount: number;
  lastAmbientImpressionAtMs: number | null;
  candidateIsAmbient: boolean;
  candidateAlreadyImpressed: boolean;
  bypassGates: boolean;
}

export interface ChatCompanionGateResult {
  allowed: boolean;
  reason: ChatCompanionGateReason;
  retryAtMs: number | null;
}

export function gateChatCompanionBubble(
  input: ChatCompanionGateInput,
): ChatCompanionGateResult {
  if (input.queuedMessageCount > 0) {
    return { allowed: false, reason: "queue_busy", retryAtMs: null };
  }
  if (input.bypassGates) {
    return { allowed: true, reason: "shown", retryAtMs: null };
  }
  if (input.candidateAlreadyImpressed) {
    return { allowed: true, reason: "shown", retryAtMs: null };
  }
  if (
    input.quietUntilMs != null &&
    Number.isFinite(input.quietUntilMs) &&
    input.nowMs < input.quietUntilMs
  ) {
    return {
      allowed: false,
      reason: "startup_quiet",
      retryAtMs: input.quietUntilMs,
    };
  }
  if (
    input.candidateIsAmbient &&
    input.lastAmbientImpressionAtMs != null &&
    Number.isFinite(input.lastAmbientImpressionAtMs)
  ) {
    const readyAtMs =
      input.lastAmbientImpressionAtMs + CHAT_COMPANION_BUBBLE_GAP_MS;
    if (input.nowMs < readyAtMs) {
      return { allowed: false, reason: "cooldown", retryAtMs: readyAtMs };
    }
  }
  return { allowed: true, reason: "shown", retryAtMs: null };
}

export function deriveChatQuietUntil(args: {
  previousQuietUntilMs: number | null;
  hadUserMessages: boolean;
  hasUserMessages: boolean;
  nowMs: number;
}): number | null {
  const firstMessageQuietUntilMs =
    !args.hadUserMessages && args.hasUserMessages
      ? args.nowMs + CHAT_COMPANION_STARTUP_QUIET_MS
      : null;
  if (args.previousQuietUntilMs == null) return firstMessageQuietUntilMs;
  if (firstMessageQuietUntilMs == null) return args.previousQuietUntilMs;
  return Math.max(args.previousQuietUntilMs, firstMessageQuietUntilMs);
}

/**
 * Opening a chat that already holds a conversation starts a quiet window so
 * the companion does not pounce the moment an existing chat is opened. Brand
 * new empty chats stay null: their quiet window starts with the first user
 * message via deriveChatQuietUntil.
 */
export function initialChatQuietUntil(args: {
  hasUserMessages: boolean;
  nowMs: number;
}): number | null {
  return args.hasUserMessages
    ? args.nowMs + CHAT_COMPANION_OPEN_QUIET_MS
    : null;
}
