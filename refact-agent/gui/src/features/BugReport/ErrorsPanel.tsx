import React from "react";
import { TriangleAlert } from "lucide-react";
import classNames from "classnames";

import { Badge, EmptyState, Icon } from "../../components/ui";
import type {
  AggregatedError,
  BugReportSourceKey,
} from "./useBugReportSources";
import styles from "./ErrorsPanel.module.css";

const SOURCE_LABELS: Record<BugReportSourceKey, string> = {
  daemon: "daemon",
  engine: "engine",
  webui: "web ui",
  ide: "ide",
};

function formatAgo(at: number): string {
  const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  return `${Math.round(minutes / 60)}h ago`;
}

export type ErrorsPanelProps = {
  errors: AggregatedError[];
  onJump: (source: BugReportSourceKey) => void;
};

export const ErrorsPanel: React.FC<ErrorsPanelProps> = ({ errors, onJump }) => {
  return (
    <div className={styles.panel}>
      <div className={styles.header}>
        <Icon icon={TriangleAlert} size="sm" tone="danger" />
        <span className={styles.title}>Latest errors</span>
        <Badge tone="danger">{errors.length}</Badge>
        <span className={styles.subtitle}>all sources</span>
      </div>
      <div className={styles.list}>
        {errors.length === 0 ? (
          <EmptyState
            description="No recent errors found in any log source."
            title="All quiet"
          />
        ) : (
          errors.map((error, index) => (
            <button
              className={styles.item}
              key={`${error.source}-${index}`}
              onClick={() => onJump(error.source)}
              type="button"
            >
              <span className={styles.itemMeta}>
                <span
                  className={classNames(
                    styles.sourceChip,
                    styles[`source-${error.source}`],
                  )}
                >
                  {SOURCE_LABELS[error.source]}
                </span>
                <span
                  className={
                    error.level === "warn"
                      ? styles.levelWarn
                      : styles.levelError
                  }
                >
                  {error.level.toUpperCase()}
                </span>
                {error.count !== undefined && error.count > 1 && (
                  <Badge
                    tone={error.level === "error" ? "danger" : "default"}
                    size="xs"
                  >
                    ×{error.count}
                  </Badge>
                )}
                {error.location && (
                  <span className={classNames(styles.location, "rf-truncate")}>
                    {error.location}
                  </span>
                )}
                {error.at !== undefined && (
                  <span className={styles.ago}>{formatAgo(error.at)}</span>
                )}
              </span>
              <span className={styles.message}>{error.message}</span>
              <span className={styles.jumpHint}>jump to log →</span>
            </button>
          ))
        )}
      </div>
    </div>
  );
};
