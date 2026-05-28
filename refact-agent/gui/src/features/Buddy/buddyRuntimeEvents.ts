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

export function isBuddyRuntimeEventVisible(
  event: BuddyRuntimeEvent | null | undefined,
  nowMs = Date.now(),
): event is BuddyRuntimeEvent {
  if (event == null) return false;
  if (event.dismissed === true) return false;
  if (event.persistent === true) return true;
  if (event.ttl_ms == null || !Number.isFinite(event.ttl_ms)) return true;
  const createdAtMs = Date.parse(event.created_at);
  if (!Number.isFinite(createdAtMs) || !Number.isFinite(nowMs)) return true;
  return nowMs <= createdAtMs + event.ttl_ms;
}

export function isErrorRuntimeEvent(event: BuddyRuntimeEvent): boolean {
  return (
    event.status === "failed" ||
    isErrorRuntimeToken(event.signal_type) ||
    isErrorRuntimeToken(event.source) ||
    isErrorRuntimeToken(event.dedupe_key ?? undefined)
  );
}
