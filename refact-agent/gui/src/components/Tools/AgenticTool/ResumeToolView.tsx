import { Badge, Button } from "../../ui";
import { humanizeIdentifier } from "../../../utils/displayNames";
import styles from "./AgenticTool.module.css";

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

function parse(content: string | null): ResumeReceipt | null {
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

export function ResumeToolView({
  content,
  pending,
  failed,
  onOpenChildTrajectory,
}: {
  content: string | null;
  pending: boolean;
  failed: boolean;
  onOpenChildTrajectory: (id: string) => void;
}) {
  if (pending)
    return <p role="status">Waiting for the child resume receipt.</p>;
  const receipt = parse(content);
  if (failed || receipt === null)
    return (
      <p role="alert">
        Resume details unavailable. This error or legacy response does not
        confirm that the child resumed.
      </p>
    );
  const rows: [string, string | number][] = [
    ["Agent", receipt.agent_id],
    ["Run generation", receipt.run_generation],
    ["Record revision", receipt.change_seq],
    ["Steps used", receipt.step_count],
    ["Child step limit", receipt.max_steps ?? "Unavailable"],
    [
      "Settings step ceiling",
      receipt.settings_subagent_max_steps_ceiling ?? "Unavailable",
    ],
  ];
  if (receipt.child_chat_id)
    rows.push(["Child trajectory", receipt.child_chat_id]);
  return (
    <section aria-label="Child resume" className={styles.section}>
      <div className={styles.metaRow}>
        <Badge tone="muted">{humanizeIdentifier(receipt.action)}</Badge>
        <Badge tone="muted">{humanizeIdentifier(receipt.status)}</Badge>
      </div>
      <p className={styles.note}>
        {receipt.action === "inspect"
          ? "Inspection only; no limits or execution state changed."
          : "Resume request accepted for the same child. Queued is not running or completed."}
      </p>
      <dl className={styles.detailGrid}>
        {rows.map(([label, value]) => (
          <div key={label} className={styles.detailItem}>
            <dt className={styles.detailLabel}>{label}</dt>
            <dd className={styles.detailValue}>{value}</dd>
          </div>
        ))}
      </dl>
      {receipt.cap_stop && (
        <section aria-label="Execution cap" className={styles.section}>
          <Badge tone="warning">Execution cap reached</Badge>
          <p>
            {humanizeIdentifier(receipt.cap_stop.key)} ·{" "}
            {humanizeIdentifier(receipt.cap_stop.scope)} ·{" "}
            {receipt.cap_stop.used} / {receipt.cap_stop.limit}
          </p>
          <p className={styles.note}>
            A cap stop is not a provider quota, queue failure or policy denial.
          </p>
        </section>
      )}
      {!receipt.resume_configuration_available && (
        <p role="status">
          Saved runtime configuration unavailable; resume cannot proceed.
        </p>
      )}
      {!receipt.original_scope_available && (
        <p role="status">
          Original execution scope unavailable; no replacement checkout is
          assumed.
        </p>
      )}
      {receipt.action === "inspect" && (
        <p className={styles.note}>
          {receipt.can_raise_and_resume
            ? "The engine reported a parent-raiseable child step cap at this revision. Raising it still requires authorization."
            : receipt.can_resume
              ? "The engine reported resume eligibility at this revision. Authorization and current ownership are checked again on execution."
              : "The engine did not report resume eligibility at this revision."}
        </p>
      )}
      <p className={styles.note}>
        Historical receipt. Unknown tool outcomes are never replayed; explicit
        reconciliation may be required. Goal budgets and permission ceilings are
        unchanged.
      </p>
      {receipt.child_chat_id && (
        <div className={styles.actions}>
          <Button
            size="sm"
            onClick={() => {
              if (receipt.child_chat_id)
                onOpenChildTrajectory(receipt.child_chat_id);
            }}
          >
            Open child trajectory
          </Button>
        </div>
      )}
    </section>
  );
}
