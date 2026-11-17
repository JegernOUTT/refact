import React from "react";
import { AlertTriangle, RotateCcw } from "lucide-react";
import { Icon, IconButton, Tooltip } from "../ui";
import type { ErrorMessage } from "../../services/refact/types";
import {
  collectErrorEntries,
  errorRecoveryStatuses,
} from "../../services/refact/recoveryPresentation";
import styles from "./ErrorMessage.module.css";

export const ErrorRepeat: React.FC<{ count: number }> = ({ count }) =>
  count > 1 ? (
    <span
      className={styles.repeat}
      data-testid="error-repeat-count"
      aria-label={`Repeated ${count} times`}
    >
      ×{count}
    </span>
  ) : null;

export type ErrorMessageCardProps = {
  errors: ErrorMessage[];
  statusHosts?: ReadonlySet<ErrorMessage>;
  liveRunId?: string | null;
  onRetryGeneration?: () => void;
};

export const ErrorMessageCard: React.FC<ErrorMessageCardProps> = ({
  errors,
  statusHosts,
  liveRunId,
  onRetryGeneration,
}) => {
  const entries = collectErrorEntries(errors, []);
  if (entries.length === 0) return null;
  const statuses = errorRecoveryStatuses(errors, statusHosts, liveRunId);
  return (
    <section
      className={`${styles.card} ${styles.errors}`}
      data-testid="error-message-card"
      aria-label="Request error"
    >
      <div className={styles.heading}>
        <Icon icon={AlertTriangle} size="sm" />
        <strong>Request failed</strong>
      </div>
      <div role="alert" className={styles.errors}>
        {entries.map(({ text, repeat }) => (
          <div className={styles.message} key={text}>
            <span>{text}</span>
            <ErrorRepeat count={repeat} />
          </div>
        ))}
      </div>
      {statuses.map((status) => (
        <div
          className={styles.status}
          data-testid="error-recovery-status"
          key={status}
        >
          {status}
        </div>
      ))}
      {onRetryGeneration && (
        <div className={styles.actions}>
          <Tooltip content="Retry">
            <IconButton
              size="sm"
              variant="ghost"
              icon={RotateCcw}
              aria-label="Retry"
              onClick={onRetryGeneration}
            />
          </Tooltip>
        </div>
      )}
    </section>
  );
};
