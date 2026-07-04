import type { CodeIntelIndexState } from "../../services/refact/types";

export function isCodeIntelIndexState(
  value: unknown,
): value is CodeIntelIndexState {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.queued === "number" &&
    typeof record.cross_file_edges === "number" &&
    typeof record.cross_file_ready === "boolean"
  );
}

export function indexStateFromResponse(
  response: unknown,
): CodeIntelIndexState | null {
  if (typeof response !== "object" || response === null) return null;
  if ("detail" in response) return null;
  if (Array.isArray(response)) {
    for (const entry of response) {
      const state = indexStateFromResponse(entry);
      if (state) return state;
    }
    return null;
  }
  const record = response as Record<string, unknown>;
  return isCodeIntelIndexState(record.index_state) ? record.index_state : null;
}
