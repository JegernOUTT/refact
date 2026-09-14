import {
  lookupAgenticFamily,
  type AgenticFamily,
  type AgenticIconKey,
} from "./agenticFamilies";

export type AgenticToolStatus = "pending" | "success" | "failed";

export type AgenticDetailRow = {
  label: string;
  value: string;
};

export type AgenticMessageBody = {
  sender: string;
  recipient: string;
  text: string;
  expectsReply: boolean;
};

export type AgenticAgentRow = {
  agentId: string;
  title: string;
  status: string | null;
  detail: string | null;
};

export type AgenticToolViewModel = {
  family: AgenticFamily;
  icon: AgenticIconKey;
  toolName: string;
  headline: string;
  subtitle: string | null;
  status: AgenticToolStatus;
  statusLabel: string | null;
  message: AgenticMessageBody | null;
  agents: AgenticAgentRow[];
  details: AgenticDetailRow[];
  childChatId: string | null;
  agentIds: string[];
  body: string | null;
  errorText: string | null;
  argsJson: string | null;
  malformedArgs: boolean;
  historical: boolean;
};

export type AgenticToolInput = {
  toolName: string | undefined;
  argumentsRaw: string | undefined;
  content: string | null;
  toolFailed?: boolean;
  hasResult: boolean;
  isBusy?: boolean;
  historical?: boolean;
};

type ArgRecord = Record<string, unknown>;

const UUID_RE =
  /\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/giu;

export function parseAgenticArgs(raw: string | undefined): {
  args: ArgRecord;
  malformed: boolean;
  pretty: string | null;
} {
  if (raw === undefined || raw.trim() === "") {
    return { args: {}, malformed: false, pretty: null };
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return { args: {}, malformed: true, pretty: raw };
    }
    const args = parsed as ArgRecord;
    return { args, malformed: false, pretty: JSON.stringify(args, null, 2) };
  } catch {
    return { args: {}, malformed: true, pretty: raw };
  }
}

function readString(args: ArgRecord, ...keys: string[]): string | null {
  for (const key of keys) {
    const value = args[key];
    if (typeof value === "string" && value.trim() !== "") return value.trim();
    if (typeof value === "number" && Number.isFinite(value)) {
      return String(value);
    }
  }
  return null;
}

function readBoolean(args: ArgRecord, ...keys: string[]): boolean {
  for (const key of keys) {
    if (args[key] === true) return true;
  }
  return false;
}

function truncate(text: string, max: number): string {
  const normalized = text.replace(/\s+/gu, " ").trim();
  if (normalized.length <= max) return normalized;
  return `${normalized.slice(0, max - 1)}…`;
}

export function shortenAgentId(value: string): string {
  const trimmed = value.trim();
  if (trimmed.length <= 12) return trimmed;
  return `…${trimmed.slice(-8)}`;
}

export function collectAgentIds(text: string | null): string[] {
  if (!text) return [];
  const matches = text.match(UUID_RE);
  if (!matches) return [];
  return Array.from(new Set(matches));
}

export function extractChildChatId(text: string | null): string | null {
  if (!text) return null;
  const match = /refact:\/\/chat\/([A-Za-z0-9-]+)/u.exec(text);
  return match ? match[1] : null;
}

function detectStatus(input: AgenticToolInput): AgenticToolStatus {
  if (input.toolFailed === true) return "failed";
  if (!input.hasResult) return "pending";
  return "success";
}

function parseOverviewRows(content: string | null): AgenticAgentRow[] {
  if (!content) return [];
  const rows: AgenticAgentRow[] = [];
  for (const line of content.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed.startsWith("- ")) continue;
    const parts = trimmed
      .slice(2)
      .split(" — ")
      .map((part) => part.trim())
      .filter(Boolean);
    if (parts.length < 2) continue;
    rows.push({
      agentId: parts[0],
      title: parts[1],
      status: parts[2] ?? null,
      detail: parts.slice(3).join(" · ") || null,
    });
  }
  return rows;
}

function parseTableRows(content: string | null): AgenticAgentRow[] {
  if (!content) return [];
  const rows: AgenticAgentRow[] = [];
  for (const line of content.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed.startsWith("|") || !trimmed.endsWith("|")) continue;
    const cells = trimmed
      .slice(1, -1)
      .split("|")
      .map((cell) => cell.trim());
    if (cells.length < 4) continue;
    if (cells.every((cell) => /^:?-{2,}:?$/u.test(cell))) continue;
    if (cells[0].toLowerCase() === "id") continue;
    rows.push({
      agentId: cells[0],
      title: cells[3],
      status: cells[2] || null,
      detail: cells.slice(4).filter(Boolean).join(" · ") || null,
    });
  }
  return rows;
}

function parseDetailLines(content: string | null): AgenticDetailRow[] {
  if (!content) return [];
  const details: AgenticDetailRow[] = [];
  for (const line of content.split("\n")) {
    const match = /^-\s+([^:]{1,40}):\s*(.+)$/u.exec(line.trim());
    if (!match) continue;
    details.push({ label: match[1].trim(), value: match[2].trim() });
  }
  return details;
}

function firstHeading(content: string | null): string | null {
  if (!content) return null;
  const match = /^#\s+(.+)$/mu.exec(content);
  return match ? match[1].trim() : null;
}

function stripStructure(content: string | null): string | null {
  if (!content) return null;
  const body = content
    .split("\n")
    .filter((line) => {
      const trimmed = line.trim();
      if (trimmed === "") return false;
      if (trimmed.startsWith("#")) return false;
      if (trimmed.startsWith("- ")) return false;
      if (trimmed.startsWith("|")) return false;
      return true;
    })
    .join("\n")
    .trim();
  return body === "" ? null : body;
}

function messageFor(
  family: AgenticFamily,
  args: ArgRecord,
  isSelfReport: boolean,
): AgenticMessageBody | null {
  if (family === "message") {
    const text = readString(args, "text", "message", "note");
    if (!text) return null;
    return {
      sender: isSelfReport ? "this agent" : "this agent",
      recipient: readString(args, "to", "target", "recipient") ?? "peers",
      text,
      expectsReply: readBoolean(args, "expects_reply"),
    };
  }
  if (family === "progress") {
    const text = readString(args, "status_line", "text", "note");
    if (!text) return null;
    return {
      sender: "this agent",
      recipient: "parent",
      text,
      expectsReply: false,
    };
  }
  return null;
}

function headlineFor(
  family: AgenticFamily,
  label: string,
  args: ArgRecord,
  content: string | null,
): string {
  switch (family) {
    case "spawn": {
      const task = readString(args, "task", "title", "prompt");
      return task ? `${label}: ${truncate(task, 80)}` : label;
    }
    case "message": {
      const to = readString(args, "to", "target", "recipient");
      return to ? `${label} → ${to}` : label;
    }
    case "progress":
      return label;
    case "inspect":
    case "control": {
      const agentId = readString(args, "agent_id", "id", "agent");
      return agentId ? `${label}: ${shortenAgentId(agentId)}` : label;
    }
    case "goal":
    case "plan": {
      const summary = readString(args, "summary", "note", "content");
      return summary ? `${label}: ${truncate(summary, 80)}` : label;
    }
    default:
      return firstHeading(content) ?? label;
  }
}

export function buildAgenticToolViewModel(
  input: AgenticToolInput,
): AgenticToolViewModel | null {
  const descriptor = lookupAgenticFamily(input.toolName);
  if (!descriptor) return null;

  const { args, malformed, pretty } = parseAgenticArgs(input.argumentsRaw);
  const content = input.content;
  const status = detectStatus(input);
  const family = descriptor.family;

  const agents =
    family === "overview"
      ? [...parseOverviewRows(content), ...parseTableRows(content)]
      : [];
  const details =
    family === "inspect" || family === "goal" || family === "plan"
      ? parseDetailLines(content)
      : [];

  const errorText =
    status === "failed" && content ? truncate(content, 400) : null;

  const subtitleParts: string[] = [];
  const reason = readString(args, "reason", "expected_result");
  if (reason) subtitleParts.push(truncate(reason, 120));
  if (family === "overview" && agents.length > 0) {
    subtitleParts.push(`${agents.length} agents`);
  }

  return {
    family,
    icon: descriptor.icon,
    toolName: input.toolName ?? descriptor.label,
    headline: headlineFor(family, descriptor.label, args, content),
    subtitle: subtitleParts.length > 0 ? subtitleParts.join(" · ") : null,
    status,
    statusLabel:
      status === "pending"
        ? input.isBusy === true
          ? "running"
          : "awaiting result"
        : status === "failed"
          ? "failed"
          : null,
    message: messageFor(family, args, family === "progress"),
    agents,
    details,
    childChatId: extractChildChatId(content),
    agentIds: collectAgentIds(content),
    body: status === "failed" ? null : stripStructure(content),
    errorText,
    argsJson: pretty,
    malformedArgs: malformed,
    historical: input.historical === true,
  };
}
