import type { DaemonEvent } from "../../../services/refact/daemon";

export const MAX_LOG_LINES = 2_000;

export function filterDaemonEvents(
  events: DaemonEvent[],
  kinds: ReadonlySet<string>,
  projectId: string | null,
): DaemonEvent[] {
  return events.filter(
    (event) =>
      (kinds.size === 0 || kinds.has(event.kind)) &&
      (projectId === null || event.project_id === projectId),
  );
}

export function timelineFollowAfterScroll(
  following: boolean,
  scrollTop: number,
): boolean {
  return following && scrollTop <= 0;
}

export function appendLogLine(
  lines: string[],
  line: string,
  paused: boolean,
  cap = MAX_LOG_LINES,
): string[] {
  if (paused) return lines;
  return [...lines, line].slice(-cap);
}

export function mergeLogLines(
  lines: string[],
  pendingLines: string[],
  cap = MAX_LOG_LINES,
): string[] {
  return [...lines, ...pendingLines].slice(-cap);
}
