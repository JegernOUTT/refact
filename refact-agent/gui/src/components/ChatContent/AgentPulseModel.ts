export type AgentPulseState =
  | "running"
  | "paused"
  | "waiting"
  | "done"
  | "error"
  | "idle"
  | "unknown";

export type AgentPulseReport = {
  cardId: string;
  cardTitle: string;
  state: string;
  stateKind: AgentPulseState;
  lastActivity: string;
  tokens: string;
  currentlyEditing: string;
  lastAssistantMessage: string;
  lastToolCall: string;
  sessionNote: string | null;
  raw: string;
};

function extractField(content: string, label: string): string | null {
  const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = content.match(
    new RegExp(`\\*\\*${escaped}:\\*\\*\\s*([^\\n]+)`, "u"),
  );
  const value = match?.[1]?.trim();
  return value ? value : null;
}

function stripBlockQuote(text: string): string {
  return text
    .split("\n")
    .map((line) => line.replace(/^> ?/u, ""))
    .join("\n")
    .trim();
}

function extractSection(content: string, heading: string): string | null {
  const escaped = heading.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = content.match(
    new RegExp(`## ${escaped}\\n([\\s\\S]*?)(?=\\n## |$)`, "u"),
  );
  const value = match?.[1]?.trim();
  return value ? value : null;
}

function stripInlineCode(text: string): string {
  const trimmed = text.trim();
  if (trimmed.startsWith("`") && trimmed.endsWith("`") && trimmed.length > 1) {
    return trimmed.slice(1, -1).trim();
  }
  return trimmed;
}

function normalizeState(state: string): AgentPulseState {
  const lower = state.toLowerCase();
  if (lower.includes("generating") || lower.includes("executing")) {
    return "running";
  }
  if (lower.includes("paused")) return "paused";
  if (lower.includes("waiting")) return "waiting";
  if (lower.includes("completed") || lower.includes("done")) return "done";
  if (lower.includes("error") || lower.includes("failed")) return "error";
  if (lower.includes("idle")) return "idle";
  return "unknown";
}

export function parseAgentPulseOutput(
  content: string,
): AgentPulseReport | null {
  const titleMatch = content.match(/^# Agent Pulse:\s*(\S+)/mu);
  if (!titleMatch) return null;

  const cardId = titleMatch[1];
  const state = extractField(content, "State") ?? "unknown";
  const assistant = extractSection(content, "Last assistant message");
  const tool = extractSection(content, "Last tool call");
  const lastActivity = extractField(content, "Last activity") ?? "unknown";
  const tokens = extractField(content, "Tokens used") ?? "unknown";
  const currentlyEditing =
    extractField(content, "Currently editing") ?? "unknown";
  const cardTitle = extractField(content, "Card") ?? cardId;

  const fieldBlockEnd = content.search(/\n## /u);
  const fieldBlock =
    fieldBlockEnd >= 0 ? content.slice(0, fieldBlockEnd) : content;
  const sessionNote = fieldBlock
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#") && !line.startsWith("**"))
    .join("\n")
    .trim();

  return {
    cardId,
    cardTitle,
    state,
    stateKind: normalizeState(state),
    lastActivity,
    tokens,
    currentlyEditing,
    lastAssistantMessage: assistant ? stripBlockQuote(assistant) : "(none)",
    lastToolCall: tool ? stripInlineCode(tool) : "(none)",
    sessionNote: sessionNote || null,
    raw: content,
  };
}
