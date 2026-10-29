import { stripAgentKindPrefix } from "../../../utils/displayNames";
import { compactText } from "./compactPresentation";
import { boardReceipt, receiptState } from "./messageReceipt";
import {
  agentLabel,
  agentStatus,
  asRecord,
  detailValue,
  emptyRow,
  facts,
  firstLine,
  headingTitle,
  num,
  parseJson,
  reportBody,
  statusWord,
  str,
  META_CHARS,
  type AgenticRow,
  type AgenticRowInput,
  type OverviewAgentRow,
} from "./agenticRowShared";
import { extractChildChatId, shortenAgentId } from "./agenticToolModel";

export function messageRow(input: AgenticRowInput): AgenticRow {
  const { args, content, extra, lookup } = input;
  const to = str(args, "to");
  const body = str(args, "text");
  const board = boardReceipt("agent_message", extra);
  const receipt = receiptState("agent_message", args, content, board);
  const outcome = board?.kind === "agent_message" ? board.outcome : null;
  const delivered =
    outcome === "delivered" ||
    outcome === "consumed" ||
    (content !== null && content.startsWith("Message delivered"));
  const notQueued =
    outcome === "duplicate" ||
    outcome === "expired" ||
    outcome === "superseded" ||
    outcome === "failed" ||
    (content !== null && content.startsWith("Message NOT queued"));
  const delivery = notQueued
    ? "Not sent"
    : receipt.label === "Question recorded"
      ? "Question sent"
      : receipt.label === "Answer recorded"
        ? "Answer recorded"
        : delivered
          ? "Delivered"
          : "Sent";
  return {
    ...emptyRow("Message"),
    separator: "arrow",
    target: to ? agentLabel(to, lookup) : null,
    meta: firstLine(body, META_CHARS),
    status: notQueued ? "failed" : "done",
    error: notQueued ? content : null,
    facts: facts([
      { label: "To", value: to ? agentLabel(to, lookup) : null },
      { label: "Delivery", value: content === null ? null : delivery },
      { label: "Question", value: receipt.question },
      { label: "Reply to", value: str(args, "reply_to") },
      {
        label: "Reply",
        value: args.expects_reply === true ? "Requested" : null,
      },
    ]),
    markdown: body,
    childChatId: to ? lookup(to)?.childChatId ?? null : null,
  };
}

const OVERVIEW_LINE =
  /^(\s*)-\s+(\S+)\s+—\s+(.+?)\s+—\s+(.+?)\s+—\s+step\s+(\d+)(?:\s+—|$)/u;

export function parseOverviewAgents(
  content: string,
  lookup: AgenticRowInput["lookup"],
): OverviewAgentRow[] {
  const rows: OverviewAgentRow[] = [];
  for (const line of content.split("\n")) {
    const match = OVERVIEW_LINE.exec(line);
    if (!match) continue;
    const [, , id, title, chip, step] = match;
    const word = chip.match(/[a-z_]+/gu)?.[0] ?? null;
    const mapped = agentStatus(word);
    const known = lookup(id);
    rows.push({
      agentId: id,
      title: agentLabel(id, () => known ?? { title, childChatId: null }),
      state: word ? word.replace(/_/gu, " ") : chip,
      status: mapped.status,
      step,
      childChatId: known?.childChatId ?? null,
    });
  }
  return rows;
}

export function overviewRow(input: AgenticRowInput): AgenticRow {
  const agents = input.content
    ? parseOverviewAgents(input.content, input.lookup)
    : [];
  const running = agents.filter(
    (agent) => agent.status === "running" || agent.status === "waiting",
  ).length;
  const finished = agents.length - running;
  const target =
    agents.length === 0
      ? "None active"
      : [
          running > 0 ? `${running} running` : null,
          finished > 0 ? `${finished} finished` : null,
        ]
          .filter(Boolean)
          .join(" · ");
  const attention = agents.filter((agent) => agent.status === "needs_you");
  return {
    ...emptyRow("Agents"),
    target: input.content === null ? null : target,
    status: attention.length > 0 ? "needs_you" : "done",
    statusLabel: attention.length > 0 ? "Approval needed" : null,
    agents: agents.length > 0 ? agents : null,
    markdown:
      agents.length === 0 && input.content !== null
        ? reportBody(input.content)
        : null,
  };
}

function waitEnvelope(content: string | null): {
  outcome: string | null;
  evidence: string | null;
} {
  const record = asRecord(parseJson(content));
  if (record && str(record, "target") === "agents")
    return {
      outcome: str(record, "outcome"),
      evidence: str(record, "evidence"),
    };
  return { outcome: null, evidence: content };
}

export function inspectRow(
  input: AgenticRowInput,
  name: "agent_status" | "agent_result" | "agent_wait",
): AgenticRow {
  const { args, lookup } = input;
  const verb =
    name === "agent_status"
      ? "Status"
      : name === "agent_result"
        ? "Result"
        : "Wait";
  const ids = Array.isArray(args.agent_ids)
    ? args.agent_ids.filter((id): id is string => typeof id === "string")
    : [];
  const agentId = str(args, "agent_id") ?? ids.at(0) ?? null;
  const wait = waitEnvelope(input.content);
  const text = wait.evidence ?? "";
  const json = asRecord(parseJson(input.content));
  const jsonTitle = str(json, "title");
  const title =
    headingTitle(text) ??
    (jsonTitle ? compactText(stripAgentKindPrefix(jsonTitle), 80) : null) ??
    (agentId ? agentLabel(agentId, lookup) : null) ??
    null;
  const target =
    ids.length > 1
      ? `${title ?? shortenAgentId(ids[0])} +${ids.length - 1}`
      : title;
  const word = statusWord(text) ?? str(json, "status");
  const mapped = agentStatus(word);
  const timedOut = wait.outcome === "timed_out" || text.startsWith("⏰");
  const interrupted = wait.outcome === "interrupted";
  const step = detailValue(text, "Step count");
  const timeout = num(args, "timeout_ms");
  const state = word ? word.replace(/_/gu, " ") : null;
  const meta =
    name === "agent_wait" && timedOut
      ? timeout !== null
        ? `timed out after ${Math.round(timeout / 1000)}s`
        : "timed out"
      : name === "agent_wait" && interrupted
        ? "interrupted"
        : [state, step !== null ? `step ${step}` : null]
            .filter((part): part is string => part !== null)
            .join(" · ") || null;
  const settled = name === "agent_wait" && (timedOut || interrupted);
  const alert =
    !settled && (mapped.status === "failed" || mapped.status === "needs_you");
  const status = alert ? mapped.status : "done";
  const statusLabel = alert ? mapped.label : null;
  const error = detailValue(text, "Error");
  return {
    ...emptyRow(verb),
    target,
    meta: compactText(meta, META_CHARS),
    status,
    statusLabel,
    facts: facts([
      { label: "State", value: word ? word.replace(/_/gu, " ") : null },
      { label: "Steps", value: step },
      { label: "Model", value: detailValue(text, "Model") },
      { label: "Tokens", value: detailValue(text, "Tokens") },
      { label: "Branch", value: detailValue(text, "Worktree branch") },
      { label: "Edited", value: detailValue(text, "Edited files") },
    ]).slice(0, 6),
    error: mapped.status === "failed" ? error : null,
    markdown: json && wait.outcome === null ? null : reportBody(text),
    childChatId:
      extractChildChatId(text) ??
      (agentId ? lookup(agentId)?.childChatId ?? null : null),
  };
}
