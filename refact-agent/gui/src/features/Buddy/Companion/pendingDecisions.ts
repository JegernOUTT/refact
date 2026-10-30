import type { AttentionRow } from "../../../services/refact/buddyCompanion";
import type { AttentionMemory } from "./AttentionCard";

export type PendingDecision = {
  rowId: string;
  chatId: string;
  targetId: string;
  sourceVersion: string | null;
  project: string;
  intent: NonNullable<AttentionMemory["intent"]>;
};
const key = (namespace: string) =>
  `refact:buddy:pending-decisions:v1:${namespace}`;
export function readPendingDecisions(namespace: string): PendingDecision[] {
  try {
    const value: unknown = JSON.parse(
      localStorage.getItem(key(namespace)) ?? "[]",
    );
    if (!Array.isArray(value)) return [];
    return value
      .filter((item: unknown): item is PendingDecision => {
        if (!item || typeof item !== "object") return false;
        const row = item as Record<string, unknown>;
        const intent = row.intent;
        return (
          ["rowId", "chatId", "targetId", "project"].every(
            (field) => typeof row[field] === "string",
          ) &&
          (row.sourceVersion === null ||
            typeof row.sourceVersion === "string") &&
          !!intent &&
          typeof intent === "object" &&
          "id" in intent &&
          typeof intent.id === "string" &&
          "engine" in intent &&
          typeof intent.engine === "string" &&
          "accepted" in intent &&
          (intent.accepted === null || typeof intent.accepted === "boolean")
        );
      })
      .slice(0, 128);
  } catch {
    return [];
  }
}
export function persistPendingDecision(
  namespace: string,
  project: string,
  row: AttentionRow,
  intent: NonNullable<AttentionMemory["intent"]>,
) {
  const records = readPendingDecisions(namespace);
  const existing = records.find((record) => record.rowId === row.id);
  if (existing && existing.intent.id !== intent.id)
    throw new Error(
      "An earlier decision for this request must be reconciled first.",
    );
  const record: PendingDecision = {
    rowId: row.id,
    chatId: row.chat_id,
    targetId: row.tool_call_id ?? "",
    sourceVersion: row.source_version,
    project,
    intent,
  };
  const next = existing
    ? records.map((value) => (value.rowId === row.id ? record : value))
    : [...records, record];
  if (next.length > 128)
    throw new Error(
      "Pending decision capacity reached. Inspect existing outcomes first.",
    );
  localStorage.setItem(key(namespace), JSON.stringify(next));
}
export function restoredDecisionRow(record: PendingDecision): AttentionRow {
  return {
    id: record.rowId,
    chat_id: record.chatId,
    tool_call_id: record.targetId,
    source_version: record.sourceVersion,
    kind: "decision_recovery",
    reason:
      "Previously submitted decision. Checking its original receipt; this request will not be sent again.",
    effect: "navigation_only",
    actionable: false,
    literal_command: null,
    display_command: null,
    cwd: null,
    question: null,
    parent_chat_id: null,
    run_generation: null,
    expires_at: null,
  };
}
