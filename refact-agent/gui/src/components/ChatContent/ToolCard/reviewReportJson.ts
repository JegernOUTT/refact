import type { BadgeTone } from "../../ui";

export type ReviewSeverity = "low" | "medium" | "high" | "critical";
export type VerificationStatus =
  | "unverified"
  | "verified"
  | "downgraded"
  | "rejected"
  | "needs_human_validation";
export type ReviewRankTier =
  | "execution_reproduced"
  | "corroborated"
  | "verified"
  | "needs_human_validation"
  | "unverified"
  | "downgraded";
export type PipelineStageStatus = "skipped" | "completed" | "failed";
export type ReviewAgentStatus = "ran" | "skipped" | "failed";

export interface ReviewScope {
  files_reviewed: string[];
  focus: string | null;
  diff_base: string | null;
}

export interface ReviewEvidence {
  kind: string;
  path: string | null;
  line1: number | null;
  line2: number | null;
  content: string;
}

export interface ReviewFinding {
  id: string;
  category: string;
  severity: ReviewSeverity;
  confidence: number;
  verification_status: VerificationStatus;
  rank_tier: ReviewRankTier;
  sources: string[];
  file: string;
  line1: number | null;
  line2: number | null;
  claim: string;
  evidence: ReviewEvidence[];
  impact: string | null;
  remediation: string | null;
  checks_performed: string[];
}

export interface ReviewPipelineStage {
  name: string;
  status: PipelineStageStatus;
  reason: string | null;
}

export interface ReviewMechanicalCheck {
  name: string;
  command: string[];
  exit_status: number;
  output_excerpt: string;
}

export interface ReviewMechanicalResult {
  passed: boolean;
  checks: ReviewMechanicalCheck[];
}

export interface ReviewAgentCoverage {
  agent: string;
  model: string | null;
  status: ReviewAgentStatus;
  reason: string | null;
  candidates: number;
  survived: number;
  duration_ms: number;
  steps: number | null;
}

export interface ReviewPipeline {
  stages: ReviewPipelineStage[];
  stopped_reason: string | null;
  mechanical: ReviewMechanicalResult | null;
  depth: string | null;
  agents: ReviewAgentCoverage[];
}

export interface ReviewReport {
  scope: ReviewScope;
  findings: ReviewFinding[];
  checks_performed: string[];
  summary: string;
  assumed_intent: string | null;
  pipeline: ReviewPipeline | null;
}

export const tierOrder: readonly ReviewRankTier[] = [
  "execution_reproduced",
  "corroborated",
  "verified",
  "needs_human_validation",
  "unverified",
  "downgraded",
];

const severities: readonly ReviewSeverity[] = [
  "low",
  "medium",
  "high",
  "critical",
];
const verificationStatuses: readonly VerificationStatus[] = [
  "unverified",
  "verified",
  "downgraded",
  "rejected",
  "needs_human_validation",
];
const stageStatuses: readonly PipelineStageStatus[] = [
  "skipped",
  "completed",
  "failed",
];
const agentStatuses: readonly ReviewAgentStatus[] = ["ran", "skipped", "failed"];

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

function nullableNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
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

function normalizeEvidence(value: unknown): ReviewEvidence | null {
  if (!isRecord(value)) return null;
  return {
    kind: stringValue(value.kind, "evidence"),
    path: nullableString(value.path),
    line1: nullableNumber(value.line1),
    line2: nullableNumber(value.line2),
    content: stringValue(value.content),
  };
}

function normalizeFinding(value: unknown, index: number): ReviewFinding | null {
  if (!isRecord(value)) return null;
  return {
    id: stringValue(value.id, `finding-${index + 1}`),
    category: stringValue(value.category, "uncategorized"),
    severity: memberOf(value.severity, severities, "low"),
    confidence: numberValue(value.confidence),
    verification_status: memberOf(
      value.verification_status,
      verificationStatuses,
      "unverified",
    ),
    rank_tier: memberOf(value.rank_tier, tierOrder, "unverified"),
    sources: stringArray(value.sources),
    file: stringValue(value.file),
    line1: nullableNumber(value.line1),
    line2: nullableNumber(value.line2),
    claim: stringValue(value.claim),
    evidence: Array.isArray(value.evidence)
      ? value.evidence
          .map(normalizeEvidence)
          .filter((item): item is ReviewEvidence => item !== null)
      : [],
    impact: nullableString(value.impact),
    remediation: nullableString(value.remediation),
    checks_performed: stringArray(value.checks_performed),
  };
}

function normalizeStage(value: unknown): ReviewPipelineStage | null {
  if (!isRecord(value)) return null;
  return {
    name: stringValue(value.name),
    status: memberOf(value.status, stageStatuses, "skipped"),
    reason: nullableString(value.reason),
  };
}

function normalizeMechanicalCheck(value: unknown): ReviewMechanicalCheck | null {
  if (!isRecord(value)) return null;
  return {
    name: stringValue(value.name),
    command: stringArray(value.command),
    exit_status: numberValue(value.exit_status),
    output_excerpt: stringValue(value.output_excerpt),
  };
}

function normalizeMechanical(value: unknown): ReviewMechanicalResult | null {
  if (!isRecord(value)) return null;
  return {
    passed: typeof value.passed === "boolean" ? value.passed : false,
    checks: Array.isArray(value.checks)
      ? value.checks
          .map(normalizeMechanicalCheck)
          .filter((item): item is ReviewMechanicalCheck => item !== null)
      : [],
  };
}

function normalizeAgent(value: unknown): ReviewAgentCoverage | null {
  if (!isRecord(value)) return null;
  return {
    agent: stringValue(value.agent),
    model: nullableString(value.model),
    status: memberOf(value.status, agentStatuses, "skipped"),
    reason: nullableString(value.reason),
    candidates: numberValue(value.candidates),
    survived: numberValue(value.survived),
    duration_ms: numberValue(value.duration_ms),
    steps: nullableNumber(value.steps),
  };
}

function normalizePipeline(value: unknown): ReviewPipeline | null {
  if (!isRecord(value)) return null;
  return {
    stages: Array.isArray(value.stages)
      ? value.stages
          .map(normalizeStage)
          .filter((item): item is ReviewPipelineStage => item !== null)
      : [],
    stopped_reason: nullableString(value.stopped_reason),
    mechanical: normalizeMechanical(value.mechanical),
    depth: nullableString(value.depth),
    agents: Array.isArray(value.agents)
      ? value.agents
          .map(normalizeAgent)
          .filter((item): item is ReviewAgentCoverage => item !== null)
      : [],
  };
}

export function extractReviewReport(content: string): ReviewReport | null {
  const blocks = Array.from(
    content.matchAll(/```json[\t ]*\r?\n([\s\S]*?)\r?\n```/g),
  );
  const match = blocks.at(-1);
  if (!match) return null;
  if (content.slice(match.index + match[0].length).trim().length > 0) return null;
  try {
    const parsed: unknown = JSON.parse(match[1]);
    if (
      !isRecord(parsed) ||
      !Array.isArray(parsed.findings) ||
      !isRecord(parsed.scope)
    ) {
      return null;
    }
    return {
      scope: {
        files_reviewed: stringArray(parsed.scope.files_reviewed),
        focus: nullableString(parsed.scope.focus),
        diff_base: nullableString(parsed.scope.diff_base),
      },
      findings: parsed.findings
        .map(normalizeFinding)
        .filter((item): item is ReviewFinding => item !== null),
      checks_performed: stringArray(parsed.checks_performed),
      summary: stringValue(parsed.summary),
      assumed_intent: nullableString(parsed.assumed_intent),
      pipeline: normalizePipeline(parsed.pipeline),
    };
  } catch {
    return null;
  }
}

export function severityTone(severity: ReviewSeverity): BadgeTone {
  if (severity === "critical" || severity === "high") return "danger";
  if (severity === "medium") return "warning";
  return "muted";
}

export function tierLabel(tier: ReviewRankTier): string {
  if (tier === "execution_reproduced") return "execution-reproduced";
  if (tier === "needs_human_validation") return "needs human validation";
  return tier;
}

export function agentStatusTone(status: ReviewAgentStatus): BadgeTone {
  if (status === "ran") return "success";
  if (status === "failed") return "danger";
  return "muted";
}
