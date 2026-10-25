export type CompactAgenticPresentation = {
  headline: string;
  stateLabel: string;
  tone: "neutral" | "active" | "positive" | "warning" | "danger";
  facts: readonly { label: string; value: string }[];
};

export type CompactTransportState =
  | "pending"
  | "running"
  | "succeeded"
  | "failed"
  | "unknown";

export type CompactAgenticInput = {
  toolName: string;
  arguments?: unknown;
  result?: unknown;
  extra?: unknown;
  transportState: CompactTransportState;
};

export const COMPACT_MAX_FACTS = 3;
export const COMPACT_HEADLINE_CHARS = 160;
export const COMPACT_STATE_CHARS = 48;
export const COMPACT_LABEL_CHARS = 48;
export const COMPACT_VALUE_CHARS = 240;

export function compactText(
  value: unknown,
  maxChars = COMPACT_VALUE_CHARS,
): string | null {
  if (
    typeof value !== "string" &&
    typeof value !== "boolean" &&
    !(typeof value === "number" && Number.isFinite(value))
  ) {
    return null;
  }
  if (!Number.isFinite(maxChars) || maxChars < 1) return null;
  const limit = Math.min(Math.floor(maxChars), COMPACT_VALUE_CHARS);
  const normalized = String(value).replace(/\s+/gu, " ").trim();
  if (normalized === "") return null;
  const characters: string[] = [];
  for (const character of normalized) {
    if (characters.length === limit) {
      characters.pop();
      return `${characters.join("")}…`;
    }
    characters.push(character);
  }
  return characters.join("");
}

export function readCompactRecord(
  value: unknown,
): Readonly<Record<string, unknown>> | null {
  let parsed: unknown = value;
  if (typeof value === "string") {
    if (value.trim() === "") return null;
    try {
      parsed = JSON.parse(value) as unknown;
    } catch {
      return null;
    }
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return null;
  }
  return parsed as Readonly<Record<string, unknown>>;
}

export function readCompactText(
  record: Readonly<Record<string, unknown>> | null,
  ...keys: string[]
): string | null {
  if (record === null) return null;
  for (const key of keys) {
    if (!Object.prototype.hasOwnProperty.call(record, key)) continue;
    const text = compactText(record[key]);
    if (text !== null) return text;
  }
  return null;
}

export function compactFacts(
  facts: readonly { label: unknown; value: unknown }[],
): CompactAgenticPresentation["facts"] {
  const shown: { label: string; value: string }[] = [];
  for (const fact of facts) {
    const label = compactText(fact.label, COMPACT_LABEL_CHARS);
    const value = compactText(fact.value);
    if (label === null || value === null) continue;
    shown.push({ label, value });
    if (shown.length === COMPACT_MAX_FACTS) break;
  }
  return shown;
}

export function compactTransportPresentation(
  state: CompactTransportState,
): Pick<CompactAgenticPresentation, "stateLabel" | "tone"> {
  switch (state) {
    case "pending":
      return { stateLabel: "Awaiting result", tone: "neutral" };
    case "running":
      return { stateLabel: "Running", tone: "active" };
    case "succeeded":
      return { stateLabel: "Response received", tone: "neutral" };
    case "failed":
      return { stateLabel: "Request failed", tone: "danger" };
    default:
      return { stateLabel: "Outcome unknown", tone: "neutral" };
  }
}
