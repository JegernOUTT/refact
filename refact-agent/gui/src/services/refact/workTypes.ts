export const workStates = [
  "queued",
  "running",
  "parked",
  "idle",
  "waiting_user",
  "completed",
  "failed",
  "cancelled",
  "interrupted",
  "unknown",
] as const;
export type WorkState = (typeof workStates)[number];

export type WorkRoot = {
  id: string;
  kind: "project" | "worktree";
  project: string;
  path?: string;
  branch?: string;
};
export type WorkRow = {
  key: { runtime: string; id: string };
  kind: string;
  state: WorkState;
  attention: boolean;
  title?: string;
  summary?: string;
  chat_id?: string;
  task_id?: string | null;
  card_id?: string | null;
  root: WorkRoot;
  current_tool?: string;
  freshness: "live" | "recent" | "stale" | "unknown";
  declared_files: string[];
  observed_files: string[];
  observed_reads: string[];
  updated_at_ms: number;
  measured_tokens: number | null;
  measured_cost_cents: number | null;
};
export type WorkPage = {
  snapshot_id: string;
  seq: number;
  project: string;
  coverage: string;
  page_unit: "work" | "worktrees";
  rows: WorkRow[];
  roots: WorkRoot[];
  total: number;
  returned: number;
  truncated: boolean;
  next_cursor?: string;
  generated_at_ms: number;
};

const object = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const strings = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every((entry) => typeof entry === "string");
const integer = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const optionalText = (value: unknown) =>
  value === undefined || value === null || typeof value === "string";

function isRoot(value: unknown): value is WorkRoot {
  return (
    object(value) &&
    typeof value.id === "string" &&
    typeof value.project === "string" &&
    (value.kind === "project" || value.kind === "worktree") &&
    optionalText(value.path) &&
    optionalText(value.branch)
  );
}
export function isWorkRow(value: unknown): value is WorkRow {
  return (
    object(value) &&
    object(value.key) &&
    typeof value.key.id === "string" &&
    typeof value.key.runtime === "string" &&
    typeof value.kind === "string" &&
    workStates.some((state) => state === value.state) &&
    typeof value.attention === "boolean" &&
    ["live", "recent", "stale", "unknown"].includes(String(value.freshness)) &&
    isRoot(value.root) &&
    strings(value.declared_files) &&
    strings(value.observed_files) &&
    strings(value.observed_reads) &&
    integer(value.updated_at_ms) &&
    [
      value.title,
      value.summary,
      value.chat_id,
      value.task_id,
      value.card_id,
      value.current_tool,
    ].every(optionalText) &&
    (value.measured_tokens === null || integer(value.measured_tokens)) &&
    (value.measured_cost_cents === null || integer(value.measured_cost_cents))
  );
}
export function parseWorkPage(value: unknown): WorkPage | null {
  if (
    !object(value) ||
    typeof value.snapshot_id !== "string" ||
    !integer(value.seq) ||
    typeof value.project !== "string" ||
    typeof value.coverage !== "string" ||
    (value.page_unit !== "work" && value.page_unit !== "worktrees") ||
    !Array.isArray(value.rows) ||
    !value.rows.every(isWorkRow) ||
    !Array.isArray(value.roots) ||
    !value.roots.every(isRoot) ||
    !integer(value.total) ||
    !integer(value.returned) ||
    !integer(value.generated_at_ms) ||
    typeof value.truncated !== "boolean" ||
    !optionalText(value.next_cursor)
  )
    return null;
  const count =
    value.page_unit === "work" ? value.rows.length : value.roots.length;
  if (count !== value.returned || count > value.total) return null;
  return value as WorkPage;
}

export function workSequence(
  previous: WorkPage | null,
  next: WorkPage,
): "accept" | "duplicate" | "reset" {
  if (!previous) return "accept";
  if (
    previous.snapshot_id !== next.snapshot_id ||
    previous.project !== next.project
  )
    return "reset";
  if (next.seq <= previous.seq) return "duplicate";
  return next.seq === previous.seq + 1 ? "accept" : "reset";
}
