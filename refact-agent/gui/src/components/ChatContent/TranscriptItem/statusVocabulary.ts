export type TranscriptStatus =
  | "running"
  | "waiting"
  | "done"
  | "failed"
  | "cancelled"
  | "needs_you"
  | "unknown";

export const TRANSCRIPT_STATUSES: readonly TranscriptStatus[] = [
  "running",
  "waiting",
  "done",
  "failed",
  "cancelled",
  "needs_you",
  "unknown",
];

export const TRANSCRIPT_STATUS_LABELS: Record<TranscriptStatus, string> = {
  running: "Running",
  waiting: "Waiting",
  done: "Done",
  failed: "Failed",
  cancelled: "Cancelled",
  needs_you: "Needs you",
  unknown: "Unknown",
};

export function transcriptStatusLabel(status: TranscriptStatus): string {
  return TRANSCRIPT_STATUS_LABELS[status];
}

export function isQuietStatus(status: TranscriptStatus): boolean {
  return status === "done";
}

export function isActiveStatus(status: TranscriptStatus): boolean {
  return status === "running";
}

const PROCESS_STATUS: Partial<Record<string, TranscriptStatus>> = {
  starting: "running",
  running: "running",
  running_in_background: "running",
  exited: "done",
  failed: "failed",
  killed: "cancelled",
  timed_out: "failed",
};

export function fromProcessStatus(
  status: string | null | undefined,
): TranscriptStatus {
  if (!status) return "unknown";
  return PROCESS_STATUS[status] ?? "unknown";
}

const AGENT_STATUS: Partial<Record<string, TranscriptStatus>> = {
  queued: "waiting",
  running: "running",
  waiting_for_approval: "needs_you",
  completed: "done",
  failed: "failed",
  cancelled: "cancelled",
  interrupted: "cancelled",
  closed: "done",
};

export function fromAgentStatus(
  status: string | null | undefined,
): TranscriptStatus {
  if (!status) return "unknown";
  return AGENT_STATUS[status] ?? "unknown";
}

const TOOL_TONE: Partial<Record<string, TranscriptStatus>> = {
  idle: "done",
  running: "running",
  streaming: "running",
  success: "done",
  error: "failed",
  pending: "waiting",
  succeeded: "done",
  failed: "failed",
  malformed: "failed",
  legacy: "done",
  ready: "done",
  neutral: "done",
  active: "running",
  positive: "done",
  warning: "needs_you",
  danger: "failed",
  default: "done",
  accent: "done",
  muted: "done",
  unknown: "unknown",
};

export function fromToolTone(
  tone: string | null | undefined,
): TranscriptStatus {
  if (!tone) return "unknown";
  return TOOL_TONE[tone] ?? "unknown";
}

const FAILURE_WORDS = new Set([
  "failed",
  "failure",
  "error",
  "errored",
  "rejected",
  "denied",
  "crashed",
  "timeout",
  "timed_out",
]);

const CANCEL_WORDS = new Set([
  "killed",
  "cancelled",
  "canceled",
  "aborted",
  "skipped",
  "interrupted",
]);

const DONE_WORDS = new Set([
  "ok",
  "success",
  "succeeded",
  "completed",
  "exited",
  "passed",
  "met",
  "fired",
  "accepted",
  "applied",
  "merged",
]);

const NEEDS_YOU_WORDS = new Set([
  "needs_work",
  "warning",
  "waiting_for_approval",
  "needs_approval",
  "awaiting_user",
  "conflict",
]);

export function fromOutcomeWord(
  word: string | null | undefined,
): TranscriptStatus {
  if (!word) return "unknown";
  const normalized = word.toLowerCase();
  if (FAILURE_WORDS.has(normalized)) return "failed";
  if (CANCEL_WORDS.has(normalized)) return "cancelled";
  if (DONE_WORDS.has(normalized)) return "done";
  if (NEEDS_YOU_WORDS.has(normalized)) return "needs_you";
  if (normalized === "running" || normalized === "starting") return "running";
  if (normalized === "queued" || normalized === "pending") return "waiting";
  return "unknown";
}
