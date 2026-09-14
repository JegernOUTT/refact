import { useState } from "react";
import { Button } from "../../ui";
import styles from "./AgenticTool.module.css";

export const PAGE_CHARS = 4096;

export function PagedText({
  label,
  body,
  testId,
}: {
  label: string;
  body: string;
  testId?: string;
}) {
  const [page, setPage] = useState(0);
  const pages = Math.max(1, Math.ceil(body.length / PAGE_CHARS));
  const index = Math.min(page, pages - 1);
  return (
    <section aria-label={label} className={styles.section}>
      <div className={styles.sectionLabel}>{label}</div>
      <div
        aria-label={`${label} content`}
        className={styles.body}
        data-testid={testId}
      >
        {body.slice(index * PAGE_CHARS, (index + 1) * PAGE_CHARS)}
      </div>
      {pages > 1 && (
        <nav aria-label={`${label} pages`} className={styles.actions}>
          <Button
            size="sm"
            disabled={index === 0}
            onClick={() => setPage(index - 1)}
          >
            Previous section
          </Button>
          <span role="status">
            Section {index + 1} of {pages}
          </span>
          <Button
            size="sm"
            disabled={index + 1 === pages}
            onClick={() => setPage(index + 1)}
          >
            Next section
          </Button>
        </nav>
      )}
    </section>
  );
}
