type CapStop = {
  key: string;
  scope: string;
  limit: number;
  used: number;
  resumable: boolean;
  run_generation: number;
};

type ResumeReceipt = {
  action: "inspect" | "resume" | "raise_and_resume";
  agent_id: string;
  child_chat_id: string | null;
  status: string;
  change_seq: number;
  run_generation: number;
  step_count: number;
  cap_stop: CapStop | null;
  can_resume: boolean;
  can_raise_and_resume: boolean;
  max_steps: number | null;
  settings_subagent_max_steps_ceiling: number | null;
  resume_configuration_available: boolean;
  original_scope_available: boolean;
};

function natural(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function capStop(value: unknown): value is CapStop {
  if (typeof value !== "object" || value === null) return false;
  const cap = value as Record<string, unknown>;
  return (
    typeof cap.key === "string" &&
    [
      "max_steps",
      "goal.max_turns",
      "goal.max_tokens",
      "goal.max_minutes",
      "goal.max_cost_cents",
      "goal.no_progress_turns",
      "context_tokens",
      "max_spawns",
    ].includes(cap.key) &&
    typeof cap.scope === "string" &&
    ["child", "root", "project", "engine"].includes(cap.scope) &&
    (cap.scope === "child" || cap.key === "max_spawns") &&
    natural(cap.limit) &&
    cap.limit > 0 &&
    natural(cap.used) &&
    cap.used >= cap.limit &&
    natural(cap.run_generation) &&
    typeof cap.resumable === "boolean"
  );
}

export function parseResumeReceipt(
  content: string | null,
): ResumeReceipt | null {
  let value: unknown;
  try {
    value = JSON.parse(content ?? "null");
  } catch {
    return null;
  }
  if (typeof value !== "object" || value === null) return null;
  const row = value as Record<string, unknown>;
  if (
    typeof row.action !== "string" ||
    !["inspect", "resume", "raise_and_resume"].includes(row.action) ||
    typeof row.agent_id !== "string" ||
    !row.agent_id ||
    !(
      row.child_chat_id === null ||
      (typeof row.child_chat_id === "string" && row.child_chat_id.length > 0)
    ) ||
    typeof row.status !== "string" ||
    ![
      "queued",
      "running",
      "waiting_for_approval",
      "completed",
      "failed",
      "cancelled",
      "interrupted",
    ].includes(row.status) ||
    !natural(row.change_seq) ||
    !natural(row.run_generation) ||
    !natural(row.step_count) ||
    !(row.cap_stop === null || capStop(row.cap_stop)) ||
    typeof row.can_resume !== "boolean" ||
    typeof row.can_raise_and_resume !== "boolean" ||
    !(row.max_steps === null || natural(row.max_steps)) ||
    !(
      row.settings_subagent_max_steps_ceiling === null ||
      natural(row.settings_subagent_max_steps_ceiling)
    ) ||
    typeof row.resume_configuration_available !== "boolean" ||
    typeof row.original_scope_available !== "boolean"
  )
    return null;
  if (
    capStop(row.cap_stop) &&
    row.cap_stop.run_generation !== row.run_generation
  )
    return null;
  return row as ResumeReceipt;
}
