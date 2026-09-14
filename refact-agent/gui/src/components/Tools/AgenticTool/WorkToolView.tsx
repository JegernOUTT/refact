import { Badge } from "../../ui";
import { isWorkRow, parseWorkPage } from "../../../services/refact/workTypes";
import { humanizeIdentifier } from "../../../utils/displayNames";
import styles from "./AgenticTool.module.css";

export function WorkToolView({
  content,
  pending,
  failed,
}: {
  content: string | null;
  pending: boolean;
  failed: boolean;
}) {
  if (pending) return <p role="status">Waiting for a Work snapshot.</p>;
  let value: unknown;
  try {
    value = JSON.parse(content ?? "null");
  } catch {
    value = null;
  }
  const page = parseWorkPage(value);
  const detail =
    typeof value === "object" &&
    value !== null &&
    "row" in value &&
    isWorkRow(value.row)
      ? value.row
      : null;
  if (failed || !page) {
    return (
      <p role="alert">
        Work details unavailable. This result is an error or a legacy response;
        no verified snapshot was returned.
      </p>
    );
  }
  return (
    <section aria-label="Work snapshot" className={styles.section}>
      {detail && (
        <section aria-label="Selected work">
          <strong>{detail.title ?? humanizeIdentifier(detail.kind)}</strong>
          <Badge tone={detail.attention ? "warning" : "muted"}>
            {humanizeIdentifier(detail.state)}
          </Badge>
          {detail.summary && <p className={styles.body}>{detail.summary}</p>}
          <p>
            {humanizeIdentifier(detail.freshness)} ·{" "}
            {detail.root.branch ?? detail.root.path ?? "Location unavailable"}
          </p>
        </section>
      )}
      <p>
        {page.returned} of {page.total}{" "}
        {page.page_unit === "work" ? "work items" : "worktrees"} ·{" "}
        {humanizeIdentifier(page.coverage)} coverage
      </p>
      <ul className={styles.agentList}>
        {page.rows.map((row) => (
          <li
            className={styles.agentRow}
            key={`${row.key.runtime}:${row.key.id}`}
          >
            <strong className={styles.agentTitle}>
              {row.title ?? humanizeIdentifier(row.kind)}
            </strong>
            <Badge tone={row.attention ? "warning" : "muted"}>
              {humanizeIdentifier(row.state)}
            </Badge>
            <span className={styles.agentDetail}>
              {humanizeIdentifier(row.freshness)} ·{" "}
              {row.root.branch ?? "Checkout not yet available"}
            </span>
          </li>
        ))}
        {page.page_unit === "worktrees" &&
          page.roots.map((root) => (
            <li className={styles.agentRow} key={root.id}>
              {root.branch ?? root.path ?? root.id}
            </li>
          ))}
      </ul>
      {!detail && page.rows.length === 0 && page.roots.length === 0 && (
        <p>
          No work observed in this snapshot; coverage is not proof that files
          are safe to overwrite.
        </p>
      )}
      {page.truncated && (
        <p>
          More items are available through the returned continuation cursor.
        </p>
      )}
      <p className={styles.note}>
        Historical tool snapshot · sequence {page.seq}. Discovery does not grant
        control authority.
      </p>
    </section>
  );
}
