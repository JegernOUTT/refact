import {
  humanizeIdentifier,
  stripAgentKindPrefix,
} from "../../../utils/displayNames";
import {
  NO_RESULT_LABEL,
  fromAgentStatus,
  transcriptStatusLabel,
  type TranscriptStatus,
  type UnrecordedResultState,
} from "../../ChatContent/TranscriptItem";
import { shortenAgentId } from "./agenticToolModel";
import { compactText, type CompactTransportState } from "./compactPresentation";

export type RecordValue = Readonly<Record<string, unknown>>;

export type AgenticRowFact = { label: string; value: string };

export type OverviewAgentRow = {
  agentId: string;
  title: string;
  state: string;
  status: TranscriptStatus;
  step: string | null;
  childChatId: string | null;
};

export type AgentLookup = (agentId: string) => {
  title: string;
  childChatId: string | null;
} | null;

export type AgenticRow = {
  verb: string;
  target: string | null;
  separator: "dot" | "arrow";
  meta: string | null;
  status: TranscriptStatus;
  statusLabel: string | null;
  facts: AgenticRowFact[];
  error: string | null;
  markdown: string | null;
  agents: OverviewAgentRow[] | null;
  criteria: string[] | null;
  childChatId: string | null;
  useFamilyView: boolean;
};

export type AgenticRowInput = {
  toolName: string;
  args: RecordValue;
  content: string | null;
  extra: RecordValue | undefined;
  transport: CompactTransportState;
  lookup: AgentLookup;
  unrecorded?: UnrecordedResultState;
};

export const TARGET_CHARS = 80;
export const META_CHARS = 60;

export function emptyRow(verb: string): AgenticRow {
  return {
    verb,
    target: null,
    separator: "dot",
    meta: null,
    status: "done",
    statusLabel: null,
    facts: [],
    error: null,
    markdown: null,
    agents: null,
    criteria: null,
    childChatId: null,
    useFamilyView: false,
  };
}

export function str(record: RecordValue | null, key: string): string | null {
  if (!record) return null;
  const value = record[key];
  return typeof value === "string" && value.trim() !== "" ? value.trim() : null;
}

export function num(record: RecordValue | null, key: string): number | null {
  if (!record) return null;
  const value = record[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function asRecord(value: unknown): RecordValue | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as RecordValue)
    : null;
}

export function parseJson(content: string | null): unknown {
  if (content === null) return null;
  try {
    return JSON.parse(content) as unknown;
  } catch {
    return null;
  }
}

export function firstLine(
  value: string | null,
  max = TARGET_CHARS,
): string | null {
  if (value === null) return null;
  const line = value
    .split("\n")
    .map((part) => part.replace(/^#+\s*/u, "").trim())
    .find((part) => part !== "");
  return line === undefined ? null : compactText(line, max);
}

export function facts(
  entries: readonly { label: string; value: string | number | null }[],
): AgenticRowFact[] {
  return entries.flatMap(({ label, value }) =>
    value === null || value === "" ? [] : [{ label, value: String(value) }],
  );
}

export function agentLabel(agentId: string, lookup: AgentLookup): string {
  if (agentId === "parent") return "Parent";
  const found = lookup(agentId);
  const title = found ? stripAgentKindPrefix(found.title) : "";
  return compactText(title, TARGET_CHARS) ?? shortenAgentId(agentId);
}

export function statusWord(text: string): string | null {
  const match = /^\s*-\s+Status:\s*(.+)$/mu.exec(text);
  if (!match) return null;
  const words = match[1].match(/[a-z_]+/gu);
  return words ? words[0] : null;
}

export function agentStatus(word: string | null): {
  status: TranscriptStatus;
  label: string | null;
} {
  if (word === null) return { status: "done", label: null };
  if (word === "paused") return { status: "waiting", label: "Paused" };
  const status = fromAgentStatus(word);
  return {
    status,
    label: status === "unknown" ? humanizeIdentifier(word) : null,
  };
}

export function detailValue(text: string, label: string): string | null {
  const escaped = label.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
  const match = new RegExp(`^\\s*-\\s+${escaped}:\\s*(.+)$`, "mu").exec(text);
  return match ? match[1].trim() : null;
}

export function headingTitle(text: string): string | null {
  const match = /^#\s+(.+)$/mu.exec(text);
  if (!match) return null;
  const title = stripAgentKindPrefix(
    match[1].replace(/^Agent Result:\s*/u, ""),
  );
  return compactText(title, TARGET_CHARS);
}

export function reportBody(text: string): string | null {
  const lines = text.split("\n");
  let index = 0;
  while (index < lines.length) {
    const line = lines[index].trim();
    if (line === "" || line.startsWith("#") || line.startsWith("- ")) {
      index += 1;
      continue;
    }
    break;
  }
  const body = lines.slice(index).join("\n").trim();
  return body === "" ? null : body;
}

export function applyTransport(
  row: AgenticRow,
  transport: CompactTransportState,
  content: string | null,
  unrecorded?: UnrecordedResultState,
): AgenticRow {
  if (
    transport === "running" ||
    transport === "pending" ||
    transport === "unknown"
  ) {
    const state: UnrecordedResultState = unrecorded ?? {
      status: transport === "running" ? "running" : "unknown",
      statusLabel: transport === "running" ? undefined : NO_RESULT_LABEL,
    };
    return {
      ...row,
      status: state.status,
      statusLabel: state.statusLabel ?? null,
    };
  }
  if (transport === "failed")
    return {
      ...row,
      status: "failed",
      statusLabel: null,
      error: content ?? "The tool call failed.",
      markdown: row.verb === "Message" ? row.markdown : null,
      agents: null,
      useFamilyView: false,
    };
  return row;
}

function isLiveStatus(status: TranscriptStatus): boolean {
  return status === "running" || status === "waiting";
}

export function settleRecordedRow(
  row: AgenticRow,
  transport: CompactTransportState,
): AgenticRow {
  if (transport !== "succeeded" || !isLiveStatus(row.status)) return row;
  const reported = (
    row.statusLabel ?? transcriptStatusLabel(row.status)
  ).toLowerCase();
  const meta =
    row.meta?.toLowerCase().includes(reported) === true
      ? row.meta
      : [row.meta, reported].filter(Boolean).join(" · ");
  return {
    ...row,
    status: "done",
    statusLabel: null,
    meta: compactText(meta, META_CHARS),
  };
}
