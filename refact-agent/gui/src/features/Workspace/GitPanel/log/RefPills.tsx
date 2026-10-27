import type { GitRef } from "../../../../services/refact/gitRead";
import { sortRefs } from "./sortRefs";
import styles from "./GitLog.module.css";

const MAX_VISIBLE = 3;

export function RefPill({ gitRef }: { gitRef: GitRef }) {
  return (
    <span
      className={styles.refPill}
      data-kind={gitRef.kind}
      title={gitRef.name}
    >
      {gitRef.name}
    </span>
  );
}

export function RefPills({ refs }: { refs: readonly GitRef[] | undefined }) {
  if (!refs || refs.length === 0) return null;
  const sorted = sortRefs(refs);
  const visible = sorted.slice(0, MAX_VISIBLE);
  const hidden = sorted.slice(MAX_VISIBLE);
  return (
    <span className={styles.refPills}>
      {visible.map((ref) => (
        <RefPill gitRef={ref} key={`${ref.kind}:${ref.name}`} />
      ))}
      {hidden.length > 0 ? (
        <span
          className={styles.refPill}
          data-kind="more"
          title={hidden.map((ref) => ref.name).join(", ")}
        >
          +{hidden.length}
        </span>
      ) : null}
    </span>
  );
}
