import { buildApiUrl, type EngineApiConfig } from "./apiUrl";
import type { PendingQuestionSet, QuestionResponse } from "./types";

export type CompanionMeta = {
  chat_id: string;
  source_chat_id: string;
  title: string;
  created_at: string;
  kind: "companion";
};
export type AttentionRow = {
  id: string;
  kind: string;
  chat_id: string;
  source_version: string | null;
  reason: string;
  effect: string;
  tool_name?: string | null;
  tool_arguments?: unknown;
  actionable: boolean;
  literal_command: string | null;
  display_command: string | null;
  cwd: string | null;
  tool_call_id: string | null;
  question: PendingQuestionSet | null;
  parent_chat_id: string | null;
  run_generation: number | null;
  expires_at: number | null;
};
export type AttentionSnapshot = {
  schema_version: number;
  project_root: string;
  engine_identity: string;
  coverage: string;
  partial: boolean;
  busy_chat_ids: string[];
  rows: AttentionRow[];
};
export type DecisionReceipt = {
  target_id: string;
  accepted: boolean | null;
  project_root: string;
  engine_identity: string;
  chat_id: string;
  client_request_id: string;
  request_identity: string;
  source_version: string;
  payload_hash: string;
  status: "queued" | "applied" | "rejected" | "stale" | "unknown";
  followup:
    | "not_requested"
    | "pending"
    | "dispatched"
    | "completed"
    | "unavailable"
    | "unknown";
  reason?: string | null;
};
export type CompanionConnection = EngineApiConfig & { apiKey?: string | null };
async function request(
  config: CompanionConnection,
  path: string,
  init?: RequestInit,
): Promise<unknown> {
  const response = await fetch(buildApiUrl(config, path), {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...(config.apiKey ? { Authorization: `Bearer ${config.apiKey}` } : {}),
    },
  });
  if (!response.ok)
    throw new Error(
      `Buddy request failed (${response.status}): ${await response.text()}`,
    );
  return (await response.json()) as unknown;
}
export async function ensureCompanion(
  config: CompanionConnection,
  sourceId: string,
) {
  const value = await request(
    config,
    `/v1/buddy/companion/${encodeURIComponent(sourceId)}`,
    { method: "POST", body: "{}" },
  );
  if (
    !isRecord(value) ||
    !strings(value, ["chat_id", "source_chat_id", "title", "created_at"]) ||
    value.kind !== "companion"
  )
    throw new Error("Invalid Buddy companion response");
  return value as CompanionMeta;
}
export async function getAttention(
  config: CompanionConnection,
  signal?: AbortSignal,
) {
  const value = await request(config, "/v1/buddy/attention", { signal });
  if (!isAttentionSnapshot(value))
    throw new Error("Invalid Buddy attention response");
  return value;
}
export function decideAttention(
  config: CompanionConnection,
  snapshot: AttentionSnapshot,
  row: AttentionRow,
  accepted: boolean,
  intentId: string,
) {
  return request(
    config,
    `/v1/buddy/approvals/${encodeURIComponent(row.chat_id)}/response`,
    {
      method: "POST",
      body: JSON.stringify({
        project_root: snapshot.project_root,
        engine_identity: snapshot.engine_identity,
        source_version: row.source_version,
        client_request_id: intentId,
        response: { kind: "tool", tool_call_id: row.tool_call_id, accepted },
        ...(!accepted ? { refusal_followup: true } : {}),
      }),
    },
  );
}
export async function getDecisionReceipt(
  config: CompanionConnection,
  project: string,
  chatId: string,
  intentId: string,
) {
  const value = await request(
    config,
    `/v1/buddy/approvals/${encodeURIComponent(
      chatId,
    )}/outcomes/${encodeURIComponent(
      intentId,
    )}?project_root=${encodeURIComponent(project)}`,
  );
  if (!isDecisionReceipt(value)) throw new Error("Invalid decision receipt");
  return value;
}

export function answerAttentionQuestion(
  config: CompanionConnection,
  project: string,
  row: AttentionRow,
  response: QuestionResponse,
  intentId: string,
) {
  return request(
    config,
    `/v1/buddy/decisions/questions/${encodeURIComponent(
      row.chat_id,
    )}/${encodeURIComponent(row.tool_call_id ?? "")}`,
    {
      method: "POST",
      body: JSON.stringify({
        project_root: project,
        asked_at_ms: row.question?.asked_at_ms,
        client_request_id: intentId,
        response,
      }),
    },
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function strings(value: Record<string, unknown>, keys: string[]) {
  return keys.every((key) => typeof value[key] === "string");
}
function nullableStrings(value: Record<string, unknown>, keys: string[]) {
  return keys.every(
    (key) => value[key] === null || typeof value[key] === "string",
  );
}
export function isAttentionSnapshot(
  value: unknown,
): value is AttentionSnapshot {
  return (
    isRecord(value) &&
    value.schema_version === 1 &&
    strings(value, ["project_root", "engine_identity", "coverage"]) &&
    typeof value.partial === "boolean" &&
    Array.isArray(value.busy_chat_ids) &&
    value.busy_chat_ids.every((id: unknown) => typeof id === "string") &&
    Array.isArray(value.rows) &&
    value.rows.every((row: unknown) => {
      if (
        !isRecord(row) ||
        !strings(row, ["id", "kind", "chat_id", "reason", "effect"]) ||
        typeof row.actionable !== "boolean" ||
        !nullableStrings(row, [
          "source_version",
          "literal_command",
          "display_command",
          "cwd",
          "tool_call_id",
          "parent_chat_id",
        ])
      )
        return false;
      if (row.expires_at !== null && typeof row.expires_at !== "number")
        return false;
      if (row.run_generation !== null && typeof row.run_generation !== "number")
        return false;
      if (row.question === null) return true;
      const question = row.question;
      return (
        isRecord(question) &&
        strings(question, ["tool_call_id", "status"]) &&
        typeof question.asked_at_ms === "number" &&
        Array.isArray(question.questions) &&
        question.questions.every(
          (q: unknown) =>
            isRecord(q) &&
            strings(q, ["id", "type", "text"]) &&
            (q.options === undefined ||
              (Array.isArray(q.options) &&
                q.options.every(
                  (option: unknown) => typeof option === "string",
                ))),
        )
      );
    })
  );
}
export function isDecisionReceipt(value: unknown): value is DecisionReceipt {
  return (
    isRecord(value) &&
    strings(value, [
      "project_root",
      "engine_identity",
      "chat_id",
      "client_request_id",
      "request_identity",
      "source_version",
      "payload_hash",
      "target_id",
      "status",
      "followup",
    ]) &&
    (value.accepted === null || typeof value.accepted === "boolean") &&
    ["queued", "applied", "rejected", "stale", "unknown"].includes(
      String(value.status),
    ) &&
    [
      "not_requested",
      "pending",
      "dispatched",
      "completed",
      "unavailable",
      "unknown",
    ].includes(String(value.followup))
  );
}
