import type { BuddyRuntimeEvent } from "./types";

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
