import type { ReactNode } from "react";
import classNames from "classnames";

import type { ProjectResource } from "../projectResource";
import styles from "../ProjectDetail.module.css";

export function Fact({
  label,
  value,
  mono,
}: {
  label: string;
  value: ReactNode;
  mono?: boolean;
}) {
  return (
    <div className={styles.fact}>
      <dt className={styles.factLabel}>{label}</dt>
      <dd
        className={classNames(styles.factValue, mono && styles.mono)}
        title={typeof value === "string" ? value : undefined}
      >
        {value}
      </dd>
    </div>
  );
}

export function ResourceView<T>({
  resource,
  errorText,
  children,
}: {
  resource: ProjectResource<T>;
  errorText: string;
  children: (data: T) => ReactNode;
}) {
  if (resource.state === "loading") {
    return <p className={styles.muted}>Loading…</p>;
  }
  if (resource.state === "error") {
    return <p className={styles.muted}>{errorText}</p>;
  }
  return <>{children(resource.data)}</>;
}
