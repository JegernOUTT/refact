import {
  isWorkRow,
  parseWorkFileOverlaps,
  parseWorkPage,
  withheldRowCount,
  workKeyText,
  type WorkFileOverlaps,
  type WorkOverlap,
  type WorkOverlapKind,
} from "../../../services/refact/workTypes";
import { humanizeIdentifier } from "../../../utils/displayNames";
import styles from "./AgenticTool.module.css";

const OVERLAP_LABELS: Record<WorkOverlapKind, string> = {
  physical: "Same file conflicts",
  advisory: "Cross-checkout overlaps",
};

function rowName(row: WorkOverlap["row"]): string {
  return row.disclosure === "withheld"
    ? "Withheld by privacy policy"
    : row.title ?? humanizeIdentifier(row.kind);
}

function overlapLine({ row }: WorkOverlap) {
  const place = row.root.branch ?? row.root.path ?? "checkout unavailable";
  return `${rowName(row)} · ${workKeyText(row.key)} · ${humanizeIdentifier(
    row.state,
  )} · ${place}`;
}

function FileOverlaps({ overlaps }: { overlaps: WorkFileOverlaps }) {
  if (overlaps.overlaps.length === 0) return null;
  return (
    <section aria-label="File overlap" className={styles.section}>
      <p>Paths checked: {overlaps.paths.join(", ")}</p>
      {(["physical", "advisory"] as const).map((kind) => {
        const matching = overlaps.overlaps.filter(
          (entry) => entry.overlap === kind,
        );
        if (matching.length === 0) return null;
        return (
          <ul
            aria-label={`${OVERLAP_LABELS[kind]} (${matching.length})`}
            className={styles.agentList}
            key={kind}
          >
            {matching.map((entry) => (
              <li
                className={styles.agentRow}
                key={`${entry.row.key.runtime}:${entry.row.key.id}`}
              >
                <span className={styles.agentState}>
                  {OVERLAP_LABELS[kind]}
                </span>
                <span className={styles.agentTitle}>{overlapLine(entry)}</span>
              </li>
            ))}
          </ul>
        );
      })}
    </section>
  );
}

export function WorkToolView({ content }: { content: string | null }) {
  let value: unknown;
  try {
    value = JSON.parse(content ?? "null");
  } catch {
    value = null;
  }
  const page = parseWorkPage(value);
  if (!page) return null;
  const detail =
    typeof value === "object" &&
    value !== null &&
    "row" in value &&
    isWorkRow(value.row)
      ? value.row
      : null;
  const overlaps = parseWorkFileOverlaps(value, page);
  const withheldCount = withheldRowCount(page);
  return (
    <section aria-label="Work snapshot" className={styles.section}>
      {detail?.summary && <p className={styles.body}>{detail.summary}</p>}
      {withheldCount !== null && (
        <p role="status" aria-label="Withheld work count">
          {withheldCount} {withheldCount === 1 ? "row was" : "rows were"}{" "}
          reduced by your privacy policy.
        </p>
      )}
      <ul className={styles.agentList}>
        {page.rows.map((row) => (
          <li
            className={styles.agentRow}
            key={`${row.key.runtime}:${row.key.id}`}
          >
            <span className={styles.agentState} data-attention={row.attention}>
              {humanizeIdentifier(row.state)}
            </span>
            <span className={styles.agentTitle}>{rowName(row)}</span>
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
      {overlaps && <FileOverlaps overlaps={overlaps} />}
      {!detail && page.rows.length === 0 && page.roots.length === 0 && (
        <p>No work observed.</p>
      )}
      {page.truncated && <p>More items are available through the cursor.</p>}
    </section>
  );
}
