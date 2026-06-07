import React from "react";
import { Text, Tooltip } from "../../components/ui";
import { Button, Surface } from "../../components/ui";
import classNames from "classnames";
import type { BuddyRuntimeEvent } from "./types";
import { formatBuddyTime, formatFailureLabel } from "./buddyUtils";
import styles from "./BuddyHome.module.css";

export type RecentBuddyError = BuddyRuntimeEvent & {
  occurrences?: number;
  dismissedAny?: boolean;
  dismissedAll?: boolean;
  relatedIds?: string[];
};

interface BuddyRecentErrorsPanelProps {
  recentErrors: RecentBuddyError[];
  onInvestigate: (event: RecentBuddyError) => void | Promise<void>;
  onDismiss: (event: RecentBuddyError) => void | Promise<void>;
}

export const BuddyRecentErrorsPanel: React.FC<BuddyRecentErrorsPanelProps> = ({
  recentErrors,
  onInvestigate,
  onDismiss,
}) => (
  <Surface
    className={classNames(styles.panel, styles.panelScroll)}
    data-testid="buddy-recent-errors-panel"
    radius="card"
    variant="surface-1"
  >
    <div className={styles.panelHeader}>
      <Text size="1" weight="bold" color="gray" className={styles.sectionLabel}>
        RECENT ERRORS
      </Text>
    </div>
    <div className={styles.scrollList}>
      {recentErrors.length === 0 && (
        <Text size="1" className={styles.emptyText}>
          No errors recorded — all clear ✨
        </Text>
      )}
      {recentErrors.map((e) => {
        const acknowledged = Boolean(e.dismissedAll ?? e.dismissed);
        const icon = acknowledged
          ? "✅"
          : e.priority === "critical"
            ? "🚨"
            : "🪲";
        const subtitle = [
          e.source,
          e.chat_id ? `chat ${e.chat_id.slice(0, 8)}` : null,
        ]
          .filter(Boolean)
          .join(" · ");
        const failureLabel = formatFailureLabel(e.failure_category);
        const detail = e.failure_summary ?? e.description ?? subtitle;
        return (
          <div
            key={e.id}
            className={classNames(
              styles.listRow,
              styles.errorRow,
              acknowledged && styles.errorRowAcknowledged,
            )}
          >
            <span className={styles.listIcon}>{icon}</span>
            <div className={styles.listContent}>
              <span className={styles.listTitle}>
                {e.title}
                {e.occurrences != null && e.occurrences > 1 && (
                  <span className={styles.countBadge}>×{e.occurrences}</span>
                )}
                {failureLabel && (
                  <span className={styles.ackBadge}>{failureLabel}</span>
                )}
                {acknowledged && (
                  <span className={styles.ackBadge}>acknowledged</span>
                )}
              </span>
              {detail && <span className={styles.listSubtitle}>{detail}</span>}
            </div>
            <div className={styles.errorActions}>
              <Tooltip content="Open a companion investigation and sniff the log crumbs">
                <Button
                  type="button"
                  size="sm"
                  variant="primary"
                  onClick={() => void onInvestigate(e)}
                >
                  Sniff logs
                </Button>
              </Tooltip>
              {!acknowledged && (
                <Tooltip content="Mark this gremlin as handled">
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    onClick={() => void onDismiss(e)}
                  >
                    Shoo
                  </Button>
                </Tooltip>
              )}
            </div>
            <span className={styles.listMeta}>
              {formatBuddyTime(e.created_at)}
            </span>
          </div>
        );
      })}
    </div>
  </Surface>
);
