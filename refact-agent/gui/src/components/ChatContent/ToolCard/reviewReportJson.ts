import type { BadgeTone } from "../../ui";

export type ReviewSeverity = "blocker" | "high" | "medium" | "low" | "note";
export type StageStatus = "ok" | "timed_out" | "failed" | "not_run";
export type ReviewOutcome = "reviewed" | "partial" | "inconclusive";

export interface ReviewScope {
  mode: string;
  requested_files: number;
  reviewed_files: number;
  files: string[];
  dropped_files: string[];
  focus: string | null;
  expansion: string | null;
  out_of_scope_findings: number;
}

export interface ReviewDiff {
  base: string | null;
  head: string | null;
  changed_files: number;
  hunks: number;
}

export interface CommandRun {
  cmd: string;
  exit: number;
}

export interface StageCoverage {
  files_read: string[];
  commands_run: CommandRun[];
  tools_unavailable: string[];
  stopped_early: string | null;
}

export interface StageRun {
  name: string;
  model: string | null;
  status: StageStatus;
  reason: string | null;
  duration_ms: number;
  findings: number;
  summary: string | null;
  trace_chat_id: string | null;
  coverage: StageCoverage;
}

export interface FindingLocation {
  file: string;
  line_start: number;
  line_end: number;
}

export interface Dispute {
  stage: string;
  reason: string;
}

export interface ReviewFinding {
  id: string;
  stage: string;
  model: string | null;
  title: string;
  severity: ReviewSeverity;
  file: string;
  line_start: number;
  line_end: number;
  claim: string;
  evidence: string;
  evidence_present: boolean;
  reproduction: string | null;
  fix: string | null;
  introduced_by_diff: boolean;
  out_of_scope: boolean;
  reported_by: string[];
  locations: FindingLocation[];
  disputed: Dispute | null;
}

export interface ReviewReport {
  depth: string;
  outcome: ReviewOutcome | null;
  scope: ReviewScope;
  diff: ReviewDiff;
  stages: StageRun[];
  findings: ReviewFinding[];
  duration_ms: number;
  duplicates_merged: number;
  scratch_dir: string | null;
}

export const severityOrder: readonly ReviewSeverity[] = [
  "blocker",
  "high",
  "medium",
  "low",
  "note",
];

const stageStatuses: readonly StageStatus[] = [
  "ok",
  "timed_out",
  "failed",
  "not_run",
];

const reviewOutcomes: readonly ReviewOutcome[] = [
  "reviewed",
  "partial",
  "inconclusive",
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringValue(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function nullableString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function numberValue(value: unknown, fallback = 0): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function booleanValue(value: unknown, fallback = false): boolean {
  return typeof value === "boolean" ? value : fallback;
}

function stringArray(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

function memberOf<T extends string>(
  value: unknown,
  values: readonly T[],
  fallback: T,
): T {
  return typeof value === "string" && values.some((item) => item === value)
    ? (value as T)
    : fallback;
}

function normalizeOutcome(value: unknown): ReviewOutcome | null {
  // Legacy payloads carry no outcome: render them as before, no banner.
  if (typeof value !== "string") return null;
  // New payloads fail closed: an unknown outcome must not look like a pass.
  return memberOf(value, reviewOutcomes, "inconclusive");
}

function normalizeCommand(value: unknown): CommandRun | null {
  if (!isRecord(value)) return null;
  return { cmd: stringValue(value.cmd), exit: numberValue(value.exit) };
}

function normalizeCoverage(value: unknown): StageCoverage {
  if (!isRecord(value)) {
    return {
      files_read: [],
      commands_run: [],
      tools_unavailable: [],
      stopped_early: null,
    };
  }
  return {
    files_read: stringArray(value.files_read),
    commands_run: Array.isArray(value.commands_run)
      ? value.commands_run
          .map(normalizeCommand)
          .filter((item): item is CommandRun => item !== null)
      : [],
    tools_unavailable: stringArray(value.tools_unavailable),
    stopped_early: nullableString(value.stopped_early),
  };
}

function normalizeStage(value: unknown): StageRun | null {
  if (!isRecord(value)) return null;
  return {
    name: stringValue(value.name),
    model: nullableString(value.model),
    status: memberOf(value.status, stageStatuses, "not_run"),
    reason: nullableString(value.reason),
    duration_ms: numberValue(value.duration_ms),
    findings: numberValue(value.findings),
    summary: nullableString(value.summary),
    trace_chat_id: nullableString(value.trace_chat_id),
    coverage: normalizeCoverage(value.coverage),
  };
}

function normalizeLocation(value: unknown): FindingLocation | null {
  if (!isRecord(value)) return null;
  return {
    file: stringValue(value.file),
    line_start: numberValue(value.line_start),
    line_end: numberValue(value.line_end),
  };
}

function normalizeDispute(value: unknown): Dispute | null {
  if (!isRecord(value)) return null;
  return {
    stage: stringValue(value.stage, "adversarial"),
    reason: stringValue(value.reason),
  };
}

function normalizeFinding(value: unknown, index: number): ReviewFinding | null {
  if (!isRecord(value)) return null;
  const lineStart = numberValue(value.line_start, 1);
  return {
    id: stringValue(value.id, `finding-${index + 1}`),
    stage: stringValue(value.stage, "unknown"),
    model: nullableString(value.model),
    title: stringValue(value.title),
    severity: memberOf(value.severity, severityOrder, "low"),
    file: stringValue(value.file),
    line_start: lineStart,
    line_end: numberValue(value.line_end, lineStart),
    claim: stringValue(value.claim),
    evidence: stringValue(value.evidence),
    evidence_present: booleanValue(value.evidence_present),
    reproduction: nullableString(value.reproduction),
    fix: nullableString(value.fix),
    introduced_by_diff: booleanValue(value.introduced_by_diff, true),
    out_of_scope: booleanValue(value.out_of_scope),
    reported_by: stringArray(value.reported_by),
    locations: Array.isArray(value.locations)
      ? value.locations
          .map(normalizeLocation)
          .filter((item): item is FindingLocation => item !== null)
      : [],
    disputed: normalizeDispute(value.disputed),
  };
}

function normalizeScope(value: unknown): ReviewScope {
  if (!isRecord(value)) {
    return {
      mode: "broad",
      requested_files: 0,
      reviewed_files: 0,
      files: [],
      dropped_files: [],
      focus: null,
      expansion: null,
      out_of_scope_findings: 0,
    };
  }
  return {
    mode: stringValue(value.mode, "broad"),
    requested_files: numberValue(value.requested_files),
    reviewed_files: numberValue(value.reviewed_files),
    files: stringArray(value.files),
    dropped_files: stringArray(value.dropped_files),
    focus: nullableString(value.focus),
    expansion: nullableString(value.expansion),
    out_of_scope_findings: numberValue(value.out_of_scope_findings),
  };
}

function normalizeDiff(value: unknown): ReviewDiff {
  if (!isRecord(value)) {
    return { base: null, head: null, changed_files: 0, hunks: 0 };
  }
  return {
    base: nullableString(value.base),
    head: nullableString(value.head),
    changed_files: numberValue(value.changed_files),
    hunks: numberValue(value.hunks),
  };
}

export function parseReviewReport(value: unknown): ReviewReport | null {
  if (!isRecord(value) || !Array.isArray(value.findings)) return null;
  if (!isRecord(value.scope) || !Array.isArray(value.stages)) return null;
  return {
    depth: stringValue(value.depth, "normal"),
    outcome: normalizeOutcome(value.outcome),
    scope: normalizeScope(value.scope),
    diff: normalizeDiff(value.diff),
    stages: value.stages
      .map(normalizeStage)
      .filter((item): item is StageRun => item !== null),
    findings: value.findings
      .map(normalizeFinding)
      .filter((item): item is ReviewFinding => item !== null),
    duration_ms: numberValue(value.duration_ms),
    duplicates_merged: numberValue(value.duplicates_merged),
    scratch_dir: nullableString(value.scratch_dir),
  };
}

export function reviewReportFromExtra(
  extra: Record<string, unknown> | undefined,
): ReviewReport | null {
  if (!extra) return null;
  return parseReviewReport(extra.review_report);
}

export function extractReviewReport(content: string): ReviewReport | null {
  const blocks = Array.from(
    content.matchAll(/```json[\t ]*\r?\n([\s\S]*?)\r?\n```/g),
  );
  const match = blocks.at(-1);
  if (!match) return null;
  try {
    return parseReviewReport(JSON.parse(match[1]));
  } catch {
    return null;
  }
}

export function isHypothesis(finding: ReviewFinding): boolean {
  if (finding.disputed !== null) return true;
  return finding.reproduction === null && !finding.evidence_present;
}

export function severityTone(severity: ReviewSeverity): BadgeTone {
  if (severity === "blocker" || severity === "high") return "danger";
  if (severity === "medium") return "warning";
  return "muted";
}

export function stageStatusTone(status: StageStatus): BadgeTone {
  if (status === "ok") return "success";
  if (status === "failed") return "danger";
  if (status === "timed_out") return "warning";
  return "muted";
}

export function stageStatusLabel(status: StageStatus): string {
  if (status === "timed_out") return "timed out";
  if (status === "not_run") return "not run";
  return status;
}
