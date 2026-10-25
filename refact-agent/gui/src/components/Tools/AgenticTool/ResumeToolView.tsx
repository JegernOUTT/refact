import { Badge, Button } from "../../ui";
import { humanizeIdentifier } from "../../../utils/displayNames";
import { parseResumeReceipt as parse } from "./resumeReceipt";
import styles from "./AgenticTool.module.css";

export function ResumeToolView({
  lifecycle = false,
  content,
  pending,
  failed,
  onOpenChildTrajectory,
}: {
  lifecycle?: boolean;
  content: string | null;
  pending: boolean;
  failed: boolean;
  onOpenChildTrajectory: (id: string) => void;
}) {
  if (pending)
    return (
      <p role="status">
        Waiting for the child {lifecycle ? "lifecycle" : "resume"} receipt.
      </p>
    );
  const receipt = parse(content, lifecycle);
  if (failed || receipt === null)
    return (
      <p role="alert">
        {lifecycle ? "Lifecycle" : "Resume"} details unavailable. This error or
        legacy response does not confirm that the child{" "}
        {lifecycle ? "changed state" : "resumed"}.
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
    <section
      aria-label={lifecycle ? "Child lifecycle" : "Child resume"}
      className={styles.section}
    >
      <div className={styles.metaRow}>
        <Badge tone="muted">{humanizeIdentifier(receipt.action)}</Badge>
        <Badge tone="muted">{humanizeIdentifier(receipt.status)}</Badge>
      </div>
      <p className={styles.note}>
        {receipt.action === "inspect"
          ? "Inspection only; no limits or execution state changed."
          : receipt.accepted === false
            ? receipt.summary ??
              "No lifecycle change was accepted. Inspect the recorded state before retrying."
            : receipt.summary
              ? receipt.summary
              : receipt.action === "close"
                ? "Close receipt; this is not a pause hold. Saved child state may be recovered later."
                : lifecycle
                  ? "Lifecycle request accepted for the same child."
                  : "Resume request accepted for the same child. Queued is not running or completed."}
      </p>
      {lifecycle && (
        <p className={styles.note}>
          Queued is not running or completed. Recovery and continuation are not
          pause-hold release.
        </p>
      )}
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
            : receipt.can_continue
              ? "The engine reported continuation eligibility at this revision. Authorization and current ownership are checked again on execution."
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
