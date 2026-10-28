import { humanizeIdentifier } from "../../utils/displayNames";
import {
  formatDurationLong,
  formatTokenCount,
} from "../../features/StatsDashboard/utils/formatters";
import type {
  BackgroundAgentMergeStatus,
  BackgroundAgentSummary,
  DeliverableStatus,
} from "../../services/refact/types";
import {
  fromAgentStatus,
  type TranscriptFact,
  type TranscriptStatus,
} from "../ChatContent/TranscriptItem";

const TERMINAL_STATUSES = new Set<BackgroundAgentSummary["status"]>([
  "completed",
  "failed",
  "cancelled",
  "interrupted",
]);

const MERGE_LABELS: Record<BackgroundAgentMergeStatus, string> = {
  pending: "Merge pending",
  merged: "Merged",
  conflict: "Conflict",
  skipped: "Not merged",
  failed: "Merge failed",
};

const MAX_FACTS = 6;

export function isTerminal(agent: BackgroundAgentSummary): boolean {
  return TERMINAL_STATUSES.has(agent.status);
}

export function pendingQuestions(agent: BackgroundAgentSummary): number {
  if (agent.pending_questions !== undefined) return agent.pending_questions;
  return (agent.questions ?? []).filter((question) => !question.answer).length;
}

export function agentTranscriptStatus(
  agent: BackgroundAgentSummary,
  statusKnown: boolean,
  launchFailed: boolean,
): { status: TranscriptStatus; label?: string } {
  if (launchFailed) return { status: "failed", label: "Request failed" };
  const questions = pendingQuestions(agent);
  if (questions > 0) {
    return {
      status: "needs_you",
      label: `${questions} ${questions === 1 ? "question" : "questions"}`,
    };
  }
  if (!statusKnown) return { status: "unknown", label: "Status unknown" };
  if (agent.status === "waiting_for_approval") {
    return { status: "needs_you", label: "Approval needed" };
  }
  const status = fromAgentStatus(agent.status);
  if (status === "unknown") return { status, label: "Status unknown" };
  return {
    status,
    label: status === "done" ? undefined : humanizeIdentifier(agent.status),
  };
}

function durationMs(agent: BackgroundAgentSummary): number | null {
  if (!agent.started_at || !agent.finished_at) return null;
  const start = Date.parse(agent.started_at);
  const end = Date.parse(agent.finished_at);
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) {
    return null;
  }
  return end - start;
}

function formatCost(agent: BackgroundAgentSummary): string | null {
  const cost = agent.cost_usd;
  if (typeof cost !== "number" || !Number.isFinite(cost) || cost <= 0) {
    return null;
  }
  return `$${cost.toFixed(2)}`;
}

export function agentMeta(agent: BackgroundAgentSummary): string | undefined {
  if (!isTerminal(agent)) {
    return agent.step_count > 0 ? `step ${agent.step_count}` : undefined;
  }
  const duration = durationMs(agent);
  const parts = [
    agent.step_count > 0
      ? `${agent.step_count} ${agent.step_count === 1 ? "step" : "steps"}`
      : null,
    duration !== null ? formatDurationLong(duration) : null,
    formatCost(agent),
  ].filter((part): part is string => part !== null);
  return parts.length > 0 ? parts.join(" · ") : undefined;
}

function deliverableFact(agent: BackgroundAgentSummary): TranscriptFact | null {
  const status: DeliverableStatus | null | undefined = agent.deliverable_status;
  if (status) {
    const details = [
      `${status.check} check`,
      ...(status.missing_sections.length > 0
        ? [`missing sections: ${status.missing_sections.join(", ")}`]
        : []),
      ...(status.schema_error ? [`output schema: ${status.schema_error}`] : []),
      ...(status.evidence_missing ? ["evidence section missing"] : []),
    ];
    return {
      label: "Verification",
      value: status.met ? "Deliverable met" : "Deliverable unmet",
      title: details.join(" · "),
    };
  }
  if (agent.deliverable) {
    return { label: "Verification", value: "Deliverable not evaluated" };
  }
  return isTerminal(agent)
    ? { label: "Verification", value: "Report unverified" }
    : null;
}

export function agentFacts(agent: BackgroundAgentSummary): TranscriptFact[] {
  const tokens = agent.tokens_used ?? 0;
  const facts: (TranscriptFact | null)[] = [
    agent.merge_status
      ? {
          label: "Status",
          value: MERGE_LABELS[agent.merge_status],
          title: agent.conflict_summary ?? undefined,
        }
      : null,
    agent.model ?? agent.model_type
      ? {
          label: "Model",
          value: agent.model ?? agent.model_type,
          title: agent.model_type ?? undefined,
        }
      : null,
    agent.worktree_branch
      ? { label: "Worktree", value: agent.worktree_branch }
      : null,
    agent.edited_files.length > 0 || isTerminal(agent)
      ? {
          label: "Files changed",
          value: agent.diff_summary
            ? `${agent.edited_files.length} · ${agent.diff_summary}`
            : String(agent.edited_files.length),
        }
      : null,
    deliverableFact(agent),
    !isTerminal(agent) && agent.current_tool
      ? { label: "Now", value: agent.current_tool }
      : null,
    tokens > 0
      ? { label: "Tokens", value: formatTokenCount(tokens).replace("K", "k") }
      : null,
  ];
  return facts
    .filter((fact): fact is TranscriptFact => fact !== null)
    .slice(0, MAX_FACTS);
}

export function agentFiles(agent: BackgroundAgentSummary): string[] {
  if (agent.edited_files.length > 0) return agent.edited_files;
  return isTerminal(agent) ? [] : agent.target_files;
}
