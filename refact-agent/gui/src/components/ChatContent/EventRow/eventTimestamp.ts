import type { EventMessage } from "../../../services/refact/types";
import { eventPayloadRecord } from "./eventPayload";

const SECONDS_TO_MS_THRESHOLD = 10_000_000_000;

function formatClock(value: unknown, milliseconds = false): string | null {
  let date: Date | null = null;

  if (typeof value === "number" && Number.isFinite(value)) {
    date = new Date(
      milliseconds || value > SECONDS_TO_MS_THRESHOLD ? value : value * 1000,
    );
  }

  if (typeof value === "string" && value.trim().length > 0) {
    const parsed = Date.parse(value);
    if (Number.isFinite(parsed)) date = new Date(parsed);
  }

  if (!date || Number.isNaN(date.getTime())) return null;

  const hours = date.getHours().toString().padStart(2, "0");
  const minutes = date.getMinutes().toString().padStart(2, "0");
  const seconds = date.getSeconds().toString().padStart(2, "0");
  return `${hours}:${minutes}:${seconds}`;
}

export function eventTimestamp(event: EventMessage): string | null {
  const payload = eventPayloadRecord(event);
  const candidates: [unknown, boolean][] = [
    [payload?.timestamp, false],
    [payload?.created_at_ms, true],
    [payload?.created_at, false],
    [payload?.at_ms, true],
    [payload?.ts, false],
    [event.extra?.timestamp, false],
    [event.extra?.created_at_ms, true],
    [event.extra?.created_at, false],
  ];

  for (const [candidate, milliseconds] of candidates) {
    const formatted = formatClock(candidate, milliseconds);
    if (formatted) return formatted;
  }

  return null;
}
