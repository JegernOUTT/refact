import React from "react";
import { Text, Tooltip } from "@radix-ui/themes";
import classNames from "classnames";
import type { BuddyRuntimeEvent } from "./types";
import styles from "./BuddyHome.module.css";

export type RecentBuddyError = BuddyRuntimeEvent & {
  occurrences?: number;
  dismissedAny?: boolean;
};

function formatTime(ts: string): string {
  if (!ts) return "";
  return new Date(ts).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

interface BuddyRecentErrorsPanelProps {
  recentErrors: RecentBuddyError[];
  onInvestigate: (event: BuddyRuntimeEvent) => void;
  onDismiss: (event: BuddyRuntimeEvent) => void;
}

export const BuddyRecentErrorsPanel: React.FC<BuddyRecentErrorsPanelProps> = ({
  recentErrors,
  onInvestigate,
  onDismiss,
}) => (
  <div
    className={classNames(styles.panel, styles.panelScroll)}
    data-testid="buddy-recent-errors-panel"
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
        const icon = e.dismissed
          ? "✅"
          : e.priority === "critical"
            ? "🚨"
            : "❌";
        const subtitle = [
          e.source,
          e.chat_id ? `chat ${e.chat_id.slice(0, 8)}` : null,
        ]
          .filter(Boolean)
          .join(" · ");
        const detail = e.description ? e.description : subtitle;
        return (
          <div
            key={e.id}
            className={classNames(
              styles.listRow,
              styles.errorRow,
              e.dismissed && styles.errorRowAcknowledged,
            )}
          >
            <span className={styles.listIcon}>{icon}</span>
            <div className={styles.listContent}>
              <span className={styles.listTitle}>
                {e.title}
                {e.occurrences != null && e.occurrences > 1 && (
                  <span className={styles.countBadge}>×{e.occurrences}</span>
                )}
                {e.dismissed && (
                  <span className={styles.ackBadge}>acknowledged</span>
                )}
              </span>
              {detail && <span className={styles.listSubtitle}>{detail}</span>}
            </div>
            <div className={styles.errorActions}>
              <Tooltip content="Open a Buddy chat to investigate this error">
                <button
                  type="button"
                  className={classNames(styles.chip, styles.chipPrimary)}
                  onClick={() => onInvestigate(e)}
                >
                  Investigate
                </button>
              </Tooltip>
              {!e.dismissed && (
                <Tooltip content="Mark as acknowledged">
                  <button
                    type="button"
                    className={classNames(styles.chip, styles.chipGhost)}
                    onClick={() => onDismiss(e)}
                  >
                    Dismiss
                  </button>
                </Tooltip>
              )}
            </div>
            <span className={styles.listMeta}>{formatTime(e.created_at)}</span>
          </div>
        );
      })}
    </div>
  </div>
);
