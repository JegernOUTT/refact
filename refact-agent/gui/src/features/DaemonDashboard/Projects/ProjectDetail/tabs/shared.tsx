import type { ReactNode } from "react";
import classNames from "classnames";

import { Button } from "../../../../../components/ui";
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
  onRetry,
  timeoutText = "Request timed out.",
  children,
}: {
  resource: ProjectResource<T>;
  errorText: string;
  onRetry?: () => void;
  timeoutText?: string;
  children: (data: T) => ReactNode;
}) {
  if (resource.state === "loading") {
    return <p className={styles.muted}>Loading…</p>;
  }
  if (resource.state === "error") {
    return (
      <div className={styles.resourceError} role="alert">
        <p className={styles.muted}>
          {resource.kind === "timeout" ? timeoutText : errorText}
        </p>
        {onRetry ? (
          <Button onClick={onRetry} size="sm" variant="soft">
            Retry
          </Button>
        ) : null}
      </div>
    );
  }
  return <>{children(resource.data)}</>;
}
