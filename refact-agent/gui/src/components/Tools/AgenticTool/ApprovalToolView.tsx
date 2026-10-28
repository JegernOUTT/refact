import { humanizeIdentifier } from "../../../utils/displayNames";
import { parseApprovalReceipt } from "./approvalReceipt";
import styles from "./AgenticTool.module.css";

export function ApprovalToolView({ content }: { content: string | null }) {
  const receipts = parseApprovalReceipt(content);
  if (receipts === null) return null;
  return (
    <section aria-label="Child command approval" className={styles.section}>
      {receipts.length === 0 ? (
        <p>No approval requests.</p>
      ) : (
        <ul className={styles.agentList}>
          {receipts.map((receipt) => (
            <li className={styles.agentRow} key={receipt.request_id}>
              <span className={styles.agentState}>
                {humanizeIdentifier(receipt.decision ?? "unknown")}
              </span>
              <span className={styles.agentTitle}>{receipt.request_id}</span>
              {receipt.agent_id && (
                <span className={styles.agentDetail}>{receipt.agent_id}</span>
              )}
              {receipt.run_generation !== undefined && (
                <span className={styles.agentDetail}>
                  run {receipt.run_generation}
                </span>
              )}
              {receipt.consumed && (
                <span className={styles.agentDetail}>consumed</span>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
