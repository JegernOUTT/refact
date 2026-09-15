import { Badge } from "../../ui";
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
import { PagedText } from "./PagedText";
import styles from "./AgenticTool.module.css";

const overlapCopy: Record<
  WorkOverlapKind,
  { label: string; description: string }
> = {
  physical: {
    label: "Same file conflicts",
    description:
      "These work items touch the same physical file. Writing it can overwrite their changes.",
  },
  advisory: {
    label: "Cross-checkout overlaps",
    description:
      "These work items touch the same repository-relative path in a separate checkout. That is an integration warning, not a file conflict and not a lock.",
  },
};

function overlapLine({ row }: WorkOverlap) {
  const name =
    row.disclosure === "withheld"
      ? "Withheld by privacy policy"
      : row.title ?? humanizeIdentifier(row.kind);
  const place = row.root.branch ?? row.root.path ?? "checkout unavailable";
  return `${name} · ${workKeyText(row.key)} · ${humanizeIdentifier(
    row.state,
  )} · ${place}`;
}

function OverlapGroup({
  kind,
  overlaps,
}: {
  kind: WorkOverlapKind;
  overlaps: WorkOverlap[];
}) {
  const matching = overlaps.filter((entry) => entry.overlap === kind);
  if (matching.length === 0) return null;
  const { label, description } = overlapCopy[kind];
  return (
    <>
      <p className={styles.note}>{description}</p>
      <PagedText
        label={`${label} (${matching.length})`}
        body={matching.map(overlapLine).join("\n")}
      />
    </>
  );
}

function FileOverlaps({ overlaps }: { overlaps: WorkFileOverlaps }) {
  if (overlaps.overlaps.length === 0) return null;
  return (
    <section aria-label="File overlap">
      <PagedText
        label={`Paths checked (${overlaps.paths.length})`}
        body={overlaps.paths.join("\n")}
      />
      <OverlapGroup kind="physical" overlaps={overlaps.overlaps} />
      <OverlapGroup kind="advisory" overlaps={overlaps.overlaps} />
    </section>
  );
}

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
  const overlaps = parseWorkFileOverlaps(value, page);
  const withheldCount = withheldRowCount(page);
  return (
    <section aria-label="Work snapshot" className={styles.section}>
      {detail && (
        <section aria-label="Selected work">
          <strong>
            {detail.disclosure === "withheld"
              ? "Work withheld by privacy policy"
              : detail.title ?? humanizeIdentifier(detail.kind)}
          </strong>
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
            <strong className={styles.agentTitle}>
              {row.disclosure === "withheld"
                ? "Work withheld by privacy policy"
                : row.title ?? humanizeIdentifier(row.kind)}
            </strong>
            <Badge tone={row.attention ? "warning" : "muted"}>
              {humanizeIdentifier(row.state)}
            </Badge>
            {row.disclosure === "withheld" && (
              <Badge tone="muted" aria-label="Withheld by privacy policy">
                Withheld
              </Badge>
            )}
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
