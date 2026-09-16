export const attentionStates = [
  "waiting",
  "answered",
  "defaulted",
  "rejected",
  "cancelled",
  "expired",
] as const;
export type AttentionState = (typeof attentionStates)[number];

export const attentionPriorities = ["low", "normal", "blocking"] as const;
export type AttentionPriority = (typeof attentionPriorities)[number];

export type AttentionEntry = {
  question_id: string;
  text: string;
  kind: string;
  priority: AttentionPriority;
  state: AttentionState;
  rank: number;
  rank_reason: string;
  blocked_runs: number;
  waiters: string[];
  asked_by: string;
  merged_waiters: string[];
  presented: boolean;
  irreversible: boolean;
  default_answer?: string;
  default_used?: string;
  project: string;
  asked_at_ms: number;
  settled_at_ms?: number;
};

export type AttentionQueue = {
  waiting: AttentionEntry[];
  settled: AttentionEntry[];
  outstanding: number;
  cap: number;
  batch_window_ms: number;
  default_timeout_secs: number;
  silent_defaults: number;
  generated_at_ms: number;
};

const object = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const strings = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every((entry) => typeof entry === "string");
const integer = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const optionalString = (value: unknown): value is string | undefined =>
  value === undefined || typeof value === "string";

export function isAttentionEntry(value: unknown): value is AttentionEntry {
  return (
    object(value) &&
    typeof value.question_id === "string" &&
    typeof value.text === "string" &&
    typeof value.kind === "string" &&
    attentionPriorities.some((priority) => priority === value.priority) &&
    attentionStates.some((state) => state === value.state) &&
    integer(value.rank) &&
    typeof value.rank_reason === "string" &&
    integer(value.blocked_runs) &&
    strings(value.waiters) &&
    typeof value.asked_by === "string" &&
    strings(value.merged_waiters) &&
    !value.merged_waiters.includes(value.asked_by) &&
    typeof value.presented === "boolean" &&
    typeof value.irreversible === "boolean" &&
    optionalString(value.default_answer) &&
    optionalString(value.default_used) &&
    typeof value.project === "string" &&
    integer(value.asked_at_ms) &&
    (value.settled_at_ms === undefined || integer(value.settled_at_ms))
  );
}

export function parseAttentionQueue(value: unknown): AttentionQueue | null {
  if (
    !object(value) ||
    !Array.isArray(value.waiting) ||
    !value.waiting.every(isAttentionEntry) ||
    !Array.isArray(value.settled) ||
    !value.settled.every(isAttentionEntry) ||
    !integer(value.outstanding) ||
    !integer(value.cap) ||
    !integer(value.batch_window_ms) ||
    !integer(value.default_timeout_secs) ||
    !integer(value.silent_defaults) ||
    !integer(value.generated_at_ms)
  )
    return null;
  // A question reported as defaulted without naming the default it applied
  // would be exactly the silent default this surface exists to prevent.
  if (
    value.settled.some(
      (entry) =>
        entry.state === "defaulted" && entry.default_used === undefined,
    )
  )
    return null;
  return value as AttentionQueue;
}

/** Settled questions nobody consented to: a default was applied instead. */
export function silentDefaults(queue: AttentionQueue): AttentionEntry[] {
  return queue.settled.filter((entry) => entry.state === "defaulted");
}

export function isSaturated(queue: AttentionQueue): boolean {
  return queue.outstanding >= queue.cap;
}
