import type { EventMessage } from "../../../services/refact/types";

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function eventPayloadRecord(
  event: EventMessage,
): Record<string, unknown> | null {
  return isRecord(event.payload) ? event.payload : null;
}

export function payloadString(
  payload: Record<string, unknown> | null,
  field: string,
): string | null {
  const value = payload?.[field];
  if (typeof value === "string") {
    const trimmed = value.trim();
    return trimmed.length > 0 ? trimmed : null;
  }
  return null;
}

export function payloadNumber(
  payload: Record<string, unknown> | null,
  field: string,
): number | null {
  const value = payload?.[field];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function payloadBoolean(
  payload: Record<string, unknown> | null,
  field: string,
): boolean | null {
  const value = payload?.[field];
  return typeof value === "boolean" ? value : null;
}

export function collapseWhitespace(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

const MS_PER_SECOND = 1000;
const MS_PER_MINUTE = 60 * MS_PER_SECOND;

export function formatDurationMs(durationMs: number): string {
  if (durationMs < MS_PER_SECOND) return `${Math.round(durationMs)}ms`;
  if (durationMs < MS_PER_MINUTE) {
    return `${(durationMs / MS_PER_SECOND).toFixed(1)}s`;
  }
  const roundedSeconds = Math.round(durationMs / MS_PER_SECOND);
  const minutes = Math.floor(roundedSeconds / 60);
  const seconds = roundedSeconds % 60;
  return `${minutes}m ${seconds}s`;
}
