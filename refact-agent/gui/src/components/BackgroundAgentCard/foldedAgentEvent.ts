import type {
  BackgroundAgentMergeStatus,
  BackgroundAgentSummary,
  DeliverableStatus,
  EventMessage,
  HandoffReport,
} from "../../services/refact/types";
import { isAgentStatus } from "../ChatContent/foldedEventIndex";

const MERGE_STATUSES: readonly BackgroundAgentMergeStatus[] = [
  "pending",
  "merged",
  "conflict",
  "skipped",
  "failed",
];

const SECTION_END = /\n\s*\n(?:Conflicts:|\(call agent_result)/;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function text(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0
    ? value.trim()
    : null;
}

function finite(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function strings(value: unknown): string[] | null {
  return Array.isArray(value) &&
    value.every((item): item is string => typeof item === "string")
    ? value
    : null;
}

function mergeStatus(value: unknown): BackgroundAgentMergeStatus | null {
  return MERGE_STATUSES.find((status) => status === value) ?? null;
}

export function eventSection(content: string, heading: string): string | null {
  const marker = `\n${heading}:\n`;
  const start = content.indexOf(marker);
  if (start === -1) return null;
  const rest = content.slice(start + marker.length);
  const end = rest.search(SECTION_END);
  return text(end === -1 ? rest : rest.slice(0, end));
}

export function agentFromFinishedEvent(
  base: BackgroundAgentSummary,
  event: EventMessage,
): BackgroundAgentSummary {
  const payload = isRecord(event.payload) ? event.payload : {};
  const status = isAgentStatus(payload.status) ? payload.status : base.status;
  const summary = eventSection(event.content, "Summary");
  const error =
    eventSection(event.content, "Error") ??
    eventSection(event.content, "Reason");
  const finishedAt = text(payload.last_update_at);
  return {
    ...base,
    status,
    title: text(payload.title) ?? base.title,
    child_chat_id: text(payload.child_chat_id) ?? base.child_chat_id,
    edited_files: strings(payload.edited_files) ?? base.edited_files,
    target_files: strings(payload.target_files) ?? base.target_files,
    diff_summary: text(payload.diff_summary) ?? base.diff_summary,
    conflict_summary: text(payload.conflict_summary) ?? base.conflict_summary,
    result_summary:
      status === "completed"
        ? summary ?? base.result_summary
        : base.result_summary,
    error: status === "completed" ? base.error : error ?? base.error,
    started_at: base.started_at ?? text(payload.created_at),
    finished_at: base.finished_at ?? finishedAt,
    model: text(payload.model) ?? base.model,
    model_type: text(payload.model_type) ?? base.model_type,
    worktree_branch: text(payload.worktree_branch) ?? base.worktree_branch,
    merge_status: mergeStatus(payload.merge_status) ?? base.merge_status,
    tokens_used: finite(payload.tokens_used) ?? base.tokens_used,
    cost_usd: finite(payload.cost_usd) ?? base.cost_usd,
    deliverable_status: isRecord(payload.deliverable_status)
      ? (payload.deliverable_status as DeliverableStatus)
      : base.deliverable_status,
    handoff: isRecord(payload.handoff)
      ? (payload.handoff as HandoffReport)
      : base.handoff,
  };
}
