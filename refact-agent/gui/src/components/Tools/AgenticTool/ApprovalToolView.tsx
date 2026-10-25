import { useState } from "react";
import { Badge, Button } from "../../ui";
import { humanizeIdentifier } from "../../../utils/displayNames";
import { parseApprovalReceipt as parse } from "./approvalReceipt";
import styles from "./AgenticTool.module.css";

export function ApprovalToolView({
  content,
  pending,
  failed,
}: {
  content: string | null;
  pending: boolean;
  failed: boolean;
}) {
  const [page, setPage] = useState(0);
  if (pending)
    return <p role="status">Waiting for the approval decision receipt.</p>;
  const receipts = parse(content);
  if (failed || receipts === null) {
    return (
      <p role="alert">
        Approval details unavailable. This error or legacy response does not
        confirm permission to execute.
      </p>
    );
  }
  const index = Math.min(page, Math.max(0, receipts.length - 1));
  const current = receipts.at(index);
  return (
    <section aria-label="Child command approval" className={styles.section}>
      {current ? (
        <>
          <div className={styles.metaRow}>
            <Badge tone="muted">Single execution only</Badge>
            <Badge tone="muted">
              {humanizeIdentifier(current.decision ?? "unknown")}
            </Badge>
          </div>
          <dl className={styles.detailGrid}>
            <div className={styles.detailItem}>
              <dt className={styles.detailLabel}>Request</dt>
              <dd className={styles.detailValue}>{current.request_id}</dd>
            </div>
            {current.agent_id && (
              <div className={styles.detailItem}>
                <dt className={styles.detailLabel}>Child</dt>
                <dd className={styles.detailValue}>{current.agent_id}</dd>
              </div>
            )}
            {current.run_generation !== undefined && (
              <div className={styles.detailItem}>
                <dt className={styles.detailLabel}>Run generation</dt>
                <dd className={styles.detailValue}>{current.run_generation}</dd>
              </div>
            )}
          </dl>
          {current.already_decided && (
            <p className={styles.note}>
              Repeated decision acknowledged; no additional execution
              authorized.
            </p>
          )}
          <p className={styles.note}>
            {current.consumed
              ? "Execution authorization consumed. This does not prove command success."
              : "A decision receipt is not an execution result. Exact command details remain in the privacy-checked approval notice."}
          </p>
        </>
      ) : (
        <p>No approval requests in this snapshot.</p>
      )}
      {receipts.length > 1 && (
        <nav aria-label="Approval requests" className={styles.actions}>
          <Button
            size="sm"
            disabled={index === 0}
            onClick={() => setPage(index - 1)}
          >
            Previous request
          </Button>
          <span role="status">
            {index + 1} of {receipts.length}
          </span>
          <Button
            size="sm"
            disabled={index + 1 === receipts.length}
            onClick={() => setPage(index + 1)}
          >
            Next request
          </Button>
        </nav>
      )}
      <p className={styles.note}>
        Historical approval snapshot. Parent permissions and current run
        identity are checked by the engine.
      </p>
    </section>
  );
}
