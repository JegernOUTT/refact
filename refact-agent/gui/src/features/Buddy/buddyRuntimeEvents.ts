import type { BuddyRuntimeEvent } from "./types";

const ERROR_RUNTIME_TOKENS = new Set([
  "error",
  "chat_error",
  "tool_failed",
  "task_failed",
  "connection_lost",
  "frontend_error",
  "llm_error",
  "model_error",
  "provider_error",
]);
const RUNTIME_EVENT_FRESHNESS_MS = 75_000;
const RUNTIME_EVENT_FUTURE_SKEW_MS = 30_000;
const MIN_EPOCH_MS = 946_684_800_000;
export const HIGH_ERROR_BUBBLE_GRACE_MS = 30_000;
export const CRITICAL_ERROR_BUBBLE_GRACE_MS = 75_000;
const DURABLE_RUNTIME_TOKENS = new Set([
  "speech_tour",
  "tour",
  "speech_milestone",
  "milestone",
  "speech_quest_accept",
  "quest_accept",
  "speech_quest_complete",
  "quest_complete",
  "speech_win",
  "win",
  "speech_suggestion",
  "suggestion",
  "speech_error_alert",
  "error_alert",
]);

function normalizeRuntimeToken(value: string | null | undefined): string {
  return (
    value
      ?.trim()
      .toLowerCase()
      .replace(/[:\s-]+/g, "_") ?? ""
  );
}

function isErrorRuntimeToken(value: string | null | undefined): boolean {
  const token = normalizeRuntimeToken(value);
  if (!token) return false;
  return (
    ERROR_RUNTIME_TOKENS.has(token) ||
    /(?:^|_)(?:error|failed|failure)(?:_|$)/.test(token)
  );
}

function isDurableRuntimeToken(value: string | null | undefined): boolean {
  const token = normalizeRuntimeToken(value);
  return token ? DURABLE_RUNTIME_TOKENS.has(token) : false;
}

function isDeliberatelyDurableRuntimeEvent(event: BuddyRuntimeEvent): boolean {
  return (
    event.bubble_policy === "durable" ||
    isDurableRuntimeToken(event.signal_type) ||
    isDurableRuntimeToken(event.source) ||
    isDurableRuntimeToken(event.dedupe_key ?? undefined)
  );
}

export function isBuddyRuntimeEventVisible(
  event: BuddyRuntimeEvent | null | undefined,
  nowMs = Date.now(),
): event is BuddyRuntimeEvent {
  if (event == null) return false;
  if (event.dismissed === true) return false;
  if (event.persistent === true) return true;
  if (isDeliberatelyDurableRuntimeEvent(event)) return true;
  const createdAtMs = Date.parse(event.created_at);
  if (!Number.isFinite(createdAtMs)) return true;
  if (!Number.isFinite(nowMs)) return false;
  if (nowMs < MIN_EPOCH_MS) return true;
  if (createdAtMs > nowMs + RUNTIME_EVENT_FUTURE_SKEW_MS) return false;
  if (event.ttl_ms == null || !Number.isFinite(event.ttl_ms)) {
    if (!isErrorRuntimeEvent(event)) return true;
    if (createdAtMs < MIN_EPOCH_MS) return true;
    return nowMs - createdAtMs <= RUNTIME_EVENT_FRESHNESS_MS;
  }
  if (createdAtMs < MIN_EPOCH_MS) return true;
  return nowMs <= createdAtMs + event.ttl_ms;
}

export function isErrorRuntimeEvent(event: BuddyRuntimeEvent): boolean {
  return (
    isErrorRuntimeToken(event.status) ||
    isErrorRuntimeToken(event.signal_type) ||
    isErrorRuntimeToken(event.source) ||
    isErrorRuntimeToken(event.dedupe_key ?? undefined)
  );
}

export function isFreshErrorWithinGrace(
  event: BuddyRuntimeEvent,
  nowMs = Date.now(),
): boolean {
  if (!isErrorRuntimeEvent(event)) return false;
  const graceMs = (() => {
    if (event.priority === "critical") return CRITICAL_ERROR_BUBBLE_GRACE_MS;
    if (event.priority === "high") return HIGH_ERROR_BUBBLE_GRACE_MS;
    return null;
  })();
  if (graceMs == null) return false;
  if (!Number.isFinite(nowMs)) return false;
  const createdAtMs = Date.parse(event.created_at);
  if (!Number.isFinite(createdAtMs)) return false;
  const ageMs = nowMs - createdAtMs;
  if (ageMs < 0) return false;
  return ageMs <= graceMs;
}
