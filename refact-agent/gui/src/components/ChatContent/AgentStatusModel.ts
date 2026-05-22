export type AgentStatusState =
  | "running"
  | "stuck"
  | "failed"
  | "done"
  | "paused";

export type AgentStatusTab = AgentStatusState | "all";
export type PriorityFilter = "all" | "P0" | "P1" | "P2";
export type AgeFilter = "all" | "15" | "60" | "240";
export type AgentAction = "pulse" | "diff" | "steer" | "pause" | "cancel";

export type AgentAlerts = {
  stuck: number;
  failed: number;
  paused: number;
};

export type AgentStatusRow = {
  priority: string;
  cardId: string;
  title: string;
  state: AgentStatusState;
  stateText: string;
  emoji: string;
  age: string;
  ageMinutes: number | null;
  lastTool: string | null;
  lastStatusUpdate: string | null;
  finalReport: string | null;
  raw: string;
};

export type AgentStatusReport = {
  alerts: AgentAlerts;
  rows: AgentStatusRow[];
  raw: string;
};

export type AgentStatusFilters = {
  tab: AgentStatusTab;
  priority: PriorityFilter;
  minAgeMinutes: number | null;
};

const EMPTY_ALERTS: AgentAlerts = { stuck: 0, failed: 0, paused: 0 };

export const DEFAULT_CANCEL_REASON = "Cancelled from agent status view.";
export const DEFAULT_PAUSE_REASON = "Paused from agent pulse view.";
export const STATUS_TABS: AgentStatusTab[] = [
  "all",
  "running",
  "stuck",
  "failed",
  "done",
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function getStringField(
  record: Record<string, unknown>,
  names: string[],
): string | null {
  for (const name of names) {
    const value = record[name];
    if (typeof value === "string" && value.trim()) return value.trim();
    if (typeof value === "number") return String(value);
  }
  return null;
}

function getNumberField(
  record: Record<string, unknown>,
  names: string[],
): number | null {
  for (const name of names) {
    const value = record[name];
    if (typeof value === "number" && Number.isFinite(value)) return value;
    if (typeof value === "string") {
      const parsed = Number(value);
      if (Number.isFinite(parsed)) return parsed;
    }
  }
  return null;
}

function normalizePriority(value: string | null): string {
  if (!value) return "P?";
  return value.trim().toUpperCase();
}

function normalizeState(emoji: string, text: string): AgentStatusState {
  const lower = `${emoji} ${text}`.toLowerCase();
  if (emoji.includes("🔴") || lower.includes("stuck")) return "stuck";
  if (emoji.includes("❌") || lower.includes("failed")) return "failed";
  if (emoji.includes("✅") || lower.includes("done")) return "done";
  if (
    emoji.includes("⏸") ||
    lower.includes("paused") ||
    lower.includes("approval")
  ) {
    return "paused";
  }
  return "running";
}

function stateEmoji(state: AgentStatusState, fallback: string): string {
  if (fallback) return fallback;
  switch (state) {
    case "stuck":
      return "🔴";
    case "failed":
      return "❌";
    case "done":
      return "✅";
    case "paused":
      return "⏸️";
    case "running":
      return "🔄";
  }
}

function stateLabel(state: AgentStatusState, text: string): string {
  if (text) return text;
  switch (state) {
    case "stuck":
      return "stuck";
    case "failed":
      return "failed";
    case "done":
      return "done";
    case "paused":
      return "paused";
    case "running":
      return "running";
  }
}

function parseAgeToMinutes(text: string | null): number | null {
  if (!text) return null;
  const lower = text.trim().toLowerCase();
  if (!lower || lower === "unknown" || lower === "?") return null;
  if (lower === "now") return 0;
  const match = lower.match(/(\d+)\s*([mhd])/u);
  if (!match) return null;
  const amount = Number(match[1]);
  if (!Number.isFinite(amount)) return null;
  switch (match[2]) {
    case "m":
      return amount;
    case "h":
      return amount * 60;
    case "d":
      return amount * 60 * 24;
    default:
      return null;
  }
}

function parseAlertsLine(line: string): AgentAlerts | null {
  const match = line.match(
    /Alerts:\s*(\d+)\s+stuck[\s\S]*?,\s*(\d+)\s+failed,\s*(\d+)\s+needing approval/u,
  );
  if (!match) return null;
  return {
    stuck: Number(match[1]),
    failed: Number(match[2]),
    paused: Number(match[3]),
  };
}

function parseCompactLine(line: string): AgentStatusRow | null {
  const parts = line
    .split("|")
    .map((part) => part.trim())
    .filter(Boolean);
  if (parts.length < 2) return null;

  const headMatch = parts[0].match(/^(\S+)\s+(\S+)\s+(\S+)\s+(.+)$/u);
  if (!headMatch) return null;

  const [, priorityRaw, emojiRaw, cardId, titleRaw] = headMatch;
  if (!/^[A-Za-z]+-[A-Za-z0-9-]+$/u.test(cardId)) return null;

  const stateChunk = parts[1].replace(/\s+/gu, " ").trim();
  const state = normalizeState(emojiRaw, stateChunk);
  let age = parts[2] ?? "unknown";
  if (state === "stuck") {
    const ageMatch = stateChunk.match(/stuck\s+(\S+)/iu);
    age = ageMatch?.[1] ?? age;
  }

  const lastPart = parts.find((part) => part.toLowerCase().startsWith("last:"));
  const lastTool = lastPart ? lastPart.replace(/^last:\s*/iu, "").trim() : null;

  return {
    priority: normalizePriority(priorityRaw),
    cardId,
    title: titleRaw.trim(),
    state,
    stateText: stateLabel(state, stateChunk),
    emoji: stateEmoji(state, emojiRaw),
    age,
    ageMinutes: parseAgeToMinutes(age),
    lastTool: lastTool && lastTool !== "?" ? lastTool : null,
    lastStatusUpdate: null,
    finalReport: null,
    raw: line,
  };
}

export function countAgentAlerts(rows: AgentStatusRow[]): AgentAlerts {
  return rows.reduce<AgentAlerts>(
    (acc, row) => {
      if (row.state === "stuck") acc.stuck += 1;
      if (row.state === "failed") acc.failed += 1;
      if (row.state === "paused") acc.paused += 1;
      return acc;
    },
    { ...EMPTY_ALERTS },
  );
}

export function mergeAgentAlerts(
  primary: AgentAlerts,
  fallback: AgentAlerts,
): AgentAlerts {
  return {
    stuck: Math.max(primary.stuck, fallback.stuck),
    failed: Math.max(primary.failed, fallback.failed),
    paused: Math.max(primary.paused, fallback.paused),
  };
}

function parseJsonRow(value: unknown): AgentStatusRow | null {
  if (!isRecord(value)) return null;
  const cardId = getStringField(value, ["card_id", "cardId", "id"]);
  if (!cardId) return null;

  const priority = normalizePriority(
    getStringField(value, ["priority", "prio", "severity"]),
  );
  const title = getStringField(value, ["title", "card_title", "cardTitle"]);
  const emoji = getStringField(value, ["emoji", "state_emoji", "stateEmoji"]);
  const stateText =
    getStringField(value, ["state", "status", "state_text", "stateText"]) ??
    "running";
  const state = normalizeState(emoji ?? "", stateText);
  const age = getStringField(value, ["age", "age_text", "ageText"]);
  const ageMinutes =
    getNumberField(value, ["age_minutes", "ageMinutes", "minutes_ago"]) ??
    parseAgeToMinutes(age);

  return {
    priority,
    cardId,
    title: title ?? cardId,
    state,
    stateText: stateLabel(state, stateText),
    emoji: stateEmoji(state, emoji ?? ""),
    age: age ?? (ageMinutes === null ? "unknown" : `${ageMinutes}m ago`),
    ageMinutes,
    lastTool: getStringField(value, [
      "last_tool",
      "lastTool",
      "last_tool_name",
    ]),
    lastStatusUpdate: getStringField(value, [
      "last_status_update",
      "lastStatusUpdate",
      "last_update",
    ]),
    finalReport: getStringField(value, ["final_report", "finalReport"]),
    raw: JSON.stringify(value),
  };
}

function parseJsonReport(content: string): AgentStatusReport | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(content);
  } catch {
    return null;
  }

  if (!isRecord(parsed)) return null;
  const rawRows = Array.isArray(parsed.agents)
    ? parsed.agents
    : Array.isArray(parsed.statuses)
      ? parsed.statuses
      : Array.isArray(parsed.rows)
        ? parsed.rows
        : null;
  if (!rawRows) return null;

  const rows = rawRows.flatMap((row) => {
    const parsedRow = parseJsonRow(row);
    return parsedRow ? [parsedRow] : [];
  });
  if (rows.length === 0) return null;

  const alertsValue = parsed.alerts;
  const alerts = isRecord(alertsValue)
    ? {
        stuck: getNumberField(alertsValue, ["stuck"]) ?? 0,
        failed: getNumberField(alertsValue, ["failed"]) ?? 0,
        paused:
          getNumberField(alertsValue, ["paused", "needing_approval"]) ?? 0,
      }
    : countAgentAlerts(rows);

  return { alerts, rows, raw: content };
}

function parseMarkdownReport(content: string): AgentStatusReport | null {
  let alerts: AgentAlerts | null = null;
  const rows: AgentStatusRow[] = [];

  for (const line of content.split("\n")) {
    const parsedAlerts = parseAlertsLine(line);
    if (parsedAlerts) alerts = parsedAlerts;

    const row = parseCompactLine(line);
    if (row) rows.push(row);
  }

  if (rows.length === 0 && alerts === null) return null;
  return {
    alerts: alerts ?? countAgentAlerts(rows),
    rows,
    raw: content,
  };
}

export function parseAgentStatusOutput(
  content: string,
): AgentStatusReport | null {
  return parseJsonReport(content) ?? parseMarkdownReport(content);
}

export function filterAgentStatusRows(
  rows: AgentStatusRow[],
  filters: AgentStatusFilters,
): AgentStatusRow[] {
  return rows.filter((row) => {
    if (filters.tab !== "all" && row.state !== filters.tab) return false;
    if (filters.priority !== "all" && row.priority !== filters.priority) {
      return false;
    }
    if (
      filters.minAgeMinutes !== null &&
      (row.ageMinutes === null || row.ageMinutes < filters.minAgeMinutes)
    ) {
      return false;
    }
    return true;
  });
}

export function formatAgentActionCommand(
  action: AgentAction,
  cardId: string,
  value?: string,
): string {
  const card = JSON.stringify(cardId);
  switch (action) {
    case "pulse":
      return `agent_pulse(card_id=${card})`;
    case "diff":
      return `agent_diff(card_id=${card})`;
    case "steer":
      return `agent_steer(card_id=${card}, message=${JSON.stringify(
        value ?? "",
      )})`;
    case "pause":
      return `pause_agent(card_id=${card}, reason=${JSON.stringify(
        value ?? DEFAULT_PAUSE_REASON,
      )})`;
    case "cancel":
      return `cancel_agent(card_id=${card}, reason=${JSON.stringify(
        value ?? DEFAULT_CANCEL_REASON,
      )})`;
  }
}
