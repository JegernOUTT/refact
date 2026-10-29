import type {
  BackgroundAgentMergeStatus,
  BackgroundAgentSummary,
  DeliverableStatus,
  EventMessage,
  HandoffReport,
} from "../../services/refact/types";
import { isAgentStatus } from "../ChatContent/foldedEventIndex";
import {
  eventPayloadRecord,
  isRecord,
  payloadNumber,
  payloadString,
} from "../ChatContent/EventRow/eventPayload";

const MERGE_STATUSES: readonly BackgroundAgentMergeStatus[] = [
  "pending",
  "merged",
  "conflict",
  "skipped",
  "failed",
];

const SECTION_END = /\n\s*\n(?:Conflicts:|\(call agent_result)/;

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
  const section = (end === -1 ? rest : rest.slice(0, end)).trim();
  return section === "" ? null : section;
}

export function agentFromFinishedEvent(
  base: BackgroundAgentSummary,
  event: EventMessage,
): BackgroundAgentSummary {
  const payload = eventPayloadRecord(event) ?? {};
  const text = (field: string) => payloadString(payload, field);
  const status = isAgentStatus(payload.status) ? payload.status : base.status;
  const summary = eventSection(event.content, "Summary");
  const error =
    eventSection(event.content, "Error") ??
    eventSection(event.content, "Reason");
  const finishedAt = text("last_update_at");
  return {
    ...base,
    status,
    title: text("title") ?? base.title,
    child_chat_id: text("child_chat_id") ?? base.child_chat_id,
    edited_files: strings(payload.edited_files) ?? base.edited_files,
    target_files: strings(payload.target_files) ?? base.target_files,
    diff_summary: text("diff_summary") ?? base.diff_summary,
    conflict_summary: text("conflict_summary") ?? base.conflict_summary,
    result_summary:
      status === "completed"
        ? summary ?? base.result_summary
        : base.result_summary,
    error: status === "completed" ? base.error : error ?? base.error,
    started_at: base.started_at ?? text("created_at"),
    finished_at: base.finished_at ?? finishedAt,
    model: text("model") ?? base.model,
    model_type: text("model_type") ?? base.model_type,
    worktree_branch: text("worktree_branch") ?? base.worktree_branch,
    merge_status: mergeStatus(payload.merge_status) ?? base.merge_status,
    tokens_used: payloadNumber(payload, "tokens_used") ?? base.tokens_used,
    cost_usd: payloadNumber(payload, "cost_usd") ?? base.cost_usd,
    deliverable_status: isRecord(payload.deliverable_status)
      ? (payload.deliverable_status as DeliverableStatus)
      : base.deliverable_status,
    handoff: isRecord(payload.handoff)
      ? (payload.handoff as HandoffReport)
      : base.handoff,
  };
}

export function agentFromLaunchEvent(
  event: EventMessage,
  fallbackTitle: string,
): BackgroundAgentSummary {
  const payload = eventPayloadRecord(event);
  const kind = payloadString(payload, "kind");
  const base: BackgroundAgentSummary = {
    agent_id: payloadString(payload, "agent_id") ?? "",
    parent_chat_id: payloadString(payload, "parent_chat_id") ?? "",
    child_chat_id: null,
    kind: kind === "delegate" ? "delegate" : "subagent",
    status: "queued",
    title: fallbackTitle,
    progress: null,
    step_count: 0,
    last_activity: null,
    target_files: [],
    edited_files: [],
    diff_summary: null,
    conflict_summary: null,
    result_summary: null,
    error: null,
    started_at: null,
    finished_at: null,
    change_seq: 0,
  };
  return agentFromFinishedEvent(base, event);
}
