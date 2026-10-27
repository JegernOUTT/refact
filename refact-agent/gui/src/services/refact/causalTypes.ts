import { SnapshotVisibilityError } from "./snapshotError";

export const causalKinds = [
  "delivery",
  "spawn",
  "completion",
  "question",
  "evidence",
  "decision",
] as const;
export type CausalKind = (typeof causalKinds)[number];

export type CausalCause = { event: string } | "unknown";

export type CausalEvent = {
  event_id: string;
  seq: number;
  kind: CausalKind;
  scope: string;
  work: { runtime: string; id: string };
  causes: CausalCause[];
  recorded_at_ms: number;
  paths?: string[];
};

export const replayDivergenceKinds = [
  "admission_order",
  "cause_not_yet_admitted",
  "gate_verdict",
] as const;
export type ReplayDivergenceKind = (typeof replayDivergenceKinds)[number];

export type ReplayDivergence = {
  kind: ReplayDivergenceKind;
  recorded?: string | boolean;
  replayed?: string | boolean;
  cause?: string;
};

export type ReplayFinding = {
  step: string;
  scope: string;
  divergence: ReplayDivergence;
};

export type ReplayReport = {
  root: string;
  steps: number;
  findings: ReplayFinding[];
  finding_count: number;
  reproduces_model_output: boolean;
  scope_note: string;
};

export type CausalPage = {
  snapshot_id: string;
  seq: number;
  project: string;
  root: string;
  events: CausalEvent[];
  total: number;
  returned: number;
  truncated: boolean;
  next_cursor?: string;
  complete: boolean;
  gaps: string[];
  gap_count: number;
  durable: boolean;
  generated_at_ms: number;
  replay: ReplayReport;
};

const object = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const strings = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every((entry) => typeof entry === "string");
const integer = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;

function isCause(value: unknown): value is CausalCause {
  if (value === "unknown") return true;
  return object(value) && typeof value.event === "string";
}

export function isCausalEvent(value: unknown): value is CausalEvent {
  return (
    object(value) &&
    typeof value.event_id === "string" &&
    integer(value.seq) &&
    causalKinds.some((kind) => kind === value.kind) &&
    typeof value.scope === "string" &&
    object(value.work) &&
    typeof value.work.runtime === "string" &&
    typeof value.work.id === "string" &&
    Array.isArray(value.causes) &&
    value.causes.every(isCause) &&
    integer(value.recorded_at_ms) &&
    (value.paths === undefined || strings(value.paths))
  );
}

function isFinding(value: unknown): value is ReplayFinding {
  if (
    !object(value) ||
    typeof value.step !== "string" ||
    typeof value.scope !== "string" ||
    !object(value.divergence)
  )
    return false;
  const { kind } = value.divergence;
  return replayDivergenceKinds.some((known) => known === kind);
}

function isReplay(value: unknown): value is ReplayReport {
  return (
    object(value) &&
    typeof value.root === "string" &&
    integer(value.steps) &&
    Array.isArray(value.findings) &&
    value.findings.every(isFinding) &&
    integer(value.finding_count) &&
    typeof value.reproduces_model_output === "boolean" &&
    typeof value.scope_note === "string"
  );
}

export function parseCausalPage(value: unknown): CausalPage | null {
  if (
    !object(value) ||
    typeof value.snapshot_id !== "string" ||
    !integer(value.seq) ||
    typeof value.project !== "string" ||
    typeof value.root !== "string" ||
    !Array.isArray(value.events) ||
    !value.events.every(isCausalEvent) ||
    !integer(value.total) ||
    !integer(value.returned) ||
    typeof value.truncated !== "boolean" ||
    (value.next_cursor !== undefined &&
      typeof value.next_cursor !== "string") ||
    typeof value.complete !== "boolean" ||
    !strings(value.gaps) ||
    !integer(value.gap_count) ||
    typeof value.durable !== "boolean" ||
    !integer(value.generated_at_ms) ||
    !isReplay(value.replay)
  )
    return null;
  if (value.events.length !== value.returned) return null;
  if (value.events.length > value.total) return null;
  // A trace claiming to be whole while reporting gaps would be the one thing
  // this surface must never render.
  if (value.complete && value.gap_count > 0) return null;
  return value as CausalPage;
}

export function causalSequence(
  previous: CausalPage | null,
  next: CausalPage,
): "accept" | "duplicate" | "reset" {
  if (!previous) return "accept";
  if (
    previous.snapshot_id !== next.snapshot_id ||
    previous.project !== next.project ||
    previous.root !== next.root
  )
    return "reset";
  if (next.seq <= previous.seq) return "duplicate";
  return next.seq === previous.seq + 1 ? "accept" : "reset";
}

export function causeIds(event: CausalEvent): string[] {
  return event.causes.flatMap((cause) =>
    cause === "unknown" ? [] : [cause.event],
  );
}

export function hasUnknownCause(event: CausalEvent): boolean {
  return event.causes.length === 0 || event.causes.includes("unknown");
}

/** Causes that name an event this page does not carry: rendered as gaps. */
export function missingCauses(page: CausalPage, event: CausalEvent): string[] {
  const held = new Set(page.events.map((held) => held.event_id));
  return causeIds(event).filter((id) => !held.has(id));
}

export function parseCausalFrame(block: string): CausalPage | null {
  let event = "";
  const data: string[] = [];
  for (const line of block.split("\n")) {
    if (line.startsWith("event:")) event = line.slice(6).trim();
    if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
  }
  if (event === "work_error")
    throw new SnapshotVisibilityError(
      "Trace visibility changed or service became unavailable",
      data.join("\n"),
    );
  if (data.length === 0) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(data.join("\n"));
  } catch {
    throw new Error("Invalid causal trace snapshot");
  }
  const page = parseCausalPage(parsed);
  if (!page || event !== "snapshot")
    throw new Error("Invalid causal trace snapshot");
  return page;
}
