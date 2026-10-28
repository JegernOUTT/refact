import { Target } from "lucide-react";

import type { GitRef } from "../../../../services/refact/gitRead";
import {
  EMPTY_REF_LABEL_CONTEXT,
  groupRefLabels,
  type RefLabel,
  type RefLabelContext,
} from "./refLabels";
import styles from "./GitLog.module.css";

export function RefPill({ label }: { label: RefLabel }) {
  const names = label.names.join(", ");
  return (
    <span
      aria-label={label.current ? `${label.text} (current branch)` : undefined}
      className={styles.refPill}
      data-current={label.current ? "true" : undefined}
      data-kind={label.kind}
      title={label.current ? `${names} (current branch)` : names}
    >
      {label.current ? (
        <Target aria-hidden="true" className={styles.refPillIcon} />
      ) : null}
      <span className={styles.refPillText}>{label.text}</span>
    </span>
  );
}

export function RefPills({
  refs,
  context = EMPTY_REF_LABEL_CONTEXT,
}: {
  refs: readonly GitRef[] | undefined;
  context?: RefLabelContext;
}) {
  if (!refs || refs.length === 0) return null;
  const { visible, hidden } = groupRefLabels(refs, context);
  return (
    <span className={styles.refPills}>
      {visible.map((label) => (
        <RefPill key={label.key} label={label} />
      ))}
      {hidden.length > 0 ? (
        <span
          className={styles.refPill}
          data-kind="more"
          title={hidden.flatMap((label) => label.names).join(", ")}
        >
          <span className={styles.refPillText}>+{hidden.length}</span>
        </span>
      ) : null}
    </span>
  );
}
