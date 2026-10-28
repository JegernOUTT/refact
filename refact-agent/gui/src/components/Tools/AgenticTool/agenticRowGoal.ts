import { goalEvidenceVerdict } from "../../../services/refact/evidenceTypes";
import { isWorkRow, parseWorkPage } from "../../../services/refact/workTypes";
import { humanizeIdentifier } from "../../../utils/displayNames";
import type { TranscriptStatus } from "../../ChatContent/TranscriptItem";
import { parseResumeReceipt } from "./resumeReceipt";
import {
  agentLabel,
  agentStatus,
  asRecord,
  emptyRow,
  facts,
  firstLine,
  num,
  parseJson,
  str,
  type AgenticRow,
  type AgenticRowInput,
} from "./agenticRowShared";

const VERDICT_STATUS: Record<
  string,
  { status: TranscriptStatus; label: string | null }
> = {
  passed: { status: "done", label: null },
  failed: { status: "failed", label: "Criteria not met" },
  blocked: { status: "needs_you", label: "Blocked" },
  unavailable: { status: "unknown", label: "Unavailable" },
  not_run: { status: "unknown", label: "Not run" },
  superseded: { status: "cancelled", label: "Superseded" },
};

function criteriaList(value: unknown): string[] | null {
  if (!Array.isArray(value)) return null;
  const list = value.flatMap((item) => {
    if (typeof item === "string") return [item];
    const record = asRecord(item);
    const text = str(record, "text");
    if (!text) return [];
    const id = str(record, "id");
    return [id ? `${id}: ${text}` : text];
  });
  return list.length > 0 ? list : null;
}

function versionMeta(version: number | null, criteria: number): string | null {
  const parts = [
    version !== null ? `v${version}` : null,
    criteria > 0 ? `${criteria} criteria` : null,
  ].filter(Boolean);
  return parts.length > 0 ? parts.join(" · ") : null;
}

export function goalRow(input: AgenticRowInput, name: string): AgenticRow {
  const { args, content, extra } = input;
  const result = asRecord(parseJson(content));
  const goal = asRecord(result?.goal);
  const body =
    str(goal, "content") ?? str(args, "content") ?? str(args, "note") ?? null;
  const criteria = criteriaList(goal?.criteria ?? args.criteria);
  const version =
    num(goal, "version") ??
    (name === "set_goal" ? num(result, "version") : null);
  const row: AgenticRow = {
    ...emptyRow("Goal"),
    target: firstLine(body) ?? humanizeIdentifier(name.replace("_goal", "")),
    meta: versionMeta(version, criteria?.length ?? 0),
    markdown: body,
    criteria,
  };
  if (name === "get_goal") {
    if (result && result.goal === null)
      return { ...row, target: "None installed", meta: null };
    const gaps = Array.isArray(goal?.gaps) ? goal.gaps.length : null;
    return {
      ...row,
      facts: facts([
        {
          label: "Status",
          value: str(goal, "status")
            ? humanizeIdentifier(str(goal, "status") ?? "")
            : null,
        },
        { label: "Updates", value: num(goal, "delta_count") },
        { label: "Turns", value: num(goal, "turns_used") },
        { label: "Tokens", value: num(goal, "tokens_used") },
        { label: "Gaps", value: gaps },
      ]),
    };
  }
  if (name === "update_goal") {
    const seq = num(result, "seq");
    return {
      ...row,
      meta: seq !== null ? `update ${seq}` : row.meta,
      statusLabel: result?.truncated === true ? "Truncated" : null,
      status: result?.truncated === true ? "needs_you" : "done",
    };
  }
  if (name === "validate_goal") {
    const verdict = goalEvidenceVerdict(extra);
    const mapped = verdict
      ? VERDICT_STATUS[verdict]
      : { status: "unknown" as const, label: "No verdict" };
    return {
      ...row,
      target: verdict === "passed" ? "Criteria met" : row.target,
      meta: null,
      status: mapped.status,
      statusLabel: mapped.label,
      markdown: content,
      criteria: null,
    };
  }
  if (name === "pause_goal" || name === "snooze_goal") {
    const minutes = num(args, "minutes");
    return {
      ...row,
      target: name === "pause_goal" ? "Paused" : "Snoozed",
      meta: minutes !== null ? `${minutes} min` : null,
      markdown: content,
      criteria: null,
    };
  }
  return row;
}

export function planRow(input: AgenticRowInput, name: string): AgenticRow {
  const { args, content } = input;
  const result = asRecord(parseJson(content));
  const plan = asRecord(result?.plan);
  const body =
    str(plan, "content") ?? str(args, "content") ?? str(args, "note") ?? null;
  const version = num(plan, "version") ?? num(result, "version");
  const seq = num(result, "seq");
  const empty = name === "get_plan" && result !== null && result.plan === null;
  return {
    ...emptyRow("Plan"),
    target: empty ? "None installed" : firstLine(body),
    meta:
      name === "update_plan" && seq !== null
        ? `update ${seq}`
        : version !== null
          ? `v${version}`
          : null,
    status: result?.truncated === true ? "needs_you" : "done",
    statusLabel: result?.truncated === true ? "Truncated" : null,
    facts: facts([
      { label: "Mode", value: str(plan, "mode") },
      { label: "Updates", value: num(plan, "delta_count") },
    ]),
    markdown: body,
  };
}

export function workRow(input: AgenticRowInput, name: string): AgenticRow {
  const value = parseJson(input.content);
  const page = parseWorkPage(value);
  const record = asRecord(value);
  const detail = isWorkRow(record?.row) ? record.row : null;
  const attention =
    detail?.attention === true ||
    (page?.rows.some((r) => r.attention) ?? false);
  const target =
    detail !== null
      ? detail.disclosure === "withheld"
        ? "Withheld"
        : detail.title ?? humanizeIdentifier(detail.kind)
      : page !== null
        ? `${page.returned} of ${page.total} ${
            page.page_unit === "work" ? "items" : "worktrees"
          }`
        : null;
  return {
    ...emptyRow(name === "work_inspect" ? "Inspect work" : "Work"),
    target,
    meta: detail
      ? humanizeIdentifier(detail.state)
      : page
        ? `${humanizeIdentifier(page.coverage)} coverage`
        : null,
    status: attention
      ? "needs_you"
      : page === null && input.content
        ? "unknown"
        : "done",
    statusLabel: attention ? "Attention" : null,
    useFamilyView: page !== null,
  };
}

export function resumeRow(input: AgenticRowInput, name: string): AgenticRow {
  const lifecycle = name === "agent_lifecycle";
  const receipt = parseResumeReceipt(input.content, lifecycle);
  const agentId = receipt?.agent_id ?? str(input.args, "agent_id");
  const action = receipt?.action ?? str(input.args, "action");
  const mapped = agentStatus(receipt?.status ?? null);
  const capped = receipt?.cap_stop !== null && receipt?.cap_stop !== undefined;
  return {
    ...emptyRow(lifecycle ? "Lifecycle" : "Resume"),
    target: agentId ? agentLabel(agentId, input.lookup) : null,
    meta: action ? humanizeIdentifier(action) : null,
    status: receipt?.accepted === false ? "failed" : mapped.status,
    statusLabel: capped ? "Cap reached" : mapped.label,
    childChatId: receipt?.child_chat_id ?? null,
    useFamilyView: receipt !== null,
  };
}
