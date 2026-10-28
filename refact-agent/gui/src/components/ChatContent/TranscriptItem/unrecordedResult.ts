import type { TranscriptStatus } from "./statusVocabulary";

export const NO_RESULT_LABEL = "No result";
export const NEEDS_APPROVAL_LABEL = "Needs approval";
export const BLOCKED_LABEL = "Blocked";

export type UnrecordedResultState = {
  status: TranscriptStatus;
  statusLabel?: string;
};

export type UnrecordedResultInput = {
  busy: boolean;
  pause?: "confirmation" | "denial" | "unknown" | null;
};

export function unrecordedResultState({
  busy,
  pause,
}: UnrecordedResultInput): UnrecordedResultState {
  if (pause === "denial")
    return { status: "needs_you", statusLabel: BLOCKED_LABEL };
  if (pause) return { status: "needs_you", statusLabel: NEEDS_APPROVAL_LABEL };
  if (busy) return { status: "running" };
  return { status: "unknown", statusLabel: NO_RESULT_LABEL };
}
