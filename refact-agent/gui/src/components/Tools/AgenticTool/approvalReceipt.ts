type ApprovalReceipt = {
  request_id: string;
  agent_id?: string;
  run_generation?: number;
  decision?: string;
  consumed?: boolean;
  already_decided?: boolean;
};

function receipt(value: unknown): value is ApprovalReceipt {
  if (typeof value !== "object" || value === null) return false;
  const row = value as Record<string, unknown>;
  return (
    typeof row.request_id === "string" &&
    (row.agent_id === undefined || typeof row.agent_id === "string") &&
    (row.run_generation === undefined ||
      (typeof row.run_generation === "number" &&
        Number.isSafeInteger(row.run_generation) &&
        row.run_generation >= 0)) &&
    (row.decision === undefined ||
      (typeof row.decision === "string" &&
        [
          "pending",
          "approve",
          "approved",
          "reject",
          "rejected",
          "cancelled",
          "expired",
        ].includes(row.decision))) &&
    (row.consumed === undefined || typeof row.consumed === "boolean") &&
    (row.already_decided === undefined ||
      typeof row.already_decided === "boolean")
  );
}

export function parseApprovalReceipt(
  content: string | null,
): ApprovalReceipt[] | null {
  let value: unknown;
  try {
    value = JSON.parse(content ?? "null");
  } catch {
    return null;
  }
  if (receipt(value)) return [value];
  if (typeof value !== "object" || value === null || !("requests" in value))
    return null;
  const rows: unknown = value.requests;
  return Array.isArray(rows) && rows.every(receipt) ? rows : null;
}
