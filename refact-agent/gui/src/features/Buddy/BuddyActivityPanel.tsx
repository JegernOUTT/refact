import React from "react";
import { Text, Tooltip } from "@radix-ui/themes";
import classNames from "classnames";
import type { BuddyActivityEntry } from "./types";
import styles from "./BuddyHome.module.css";

function formatTime(ts: string): string {
  if (!ts) return "";
  return new Date(ts).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

interface BuddyActivityPanelProps {
  activities: BuddyActivityEntry[];
}

export const BuddyActivityPanel: React.FC<BuddyActivityPanelProps> = ({
  activities,
}) => (
  <div
    className={classNames(styles.panel, styles.panelScroll)}
    data-testid="buddy-activity-panel"
  >
    <div className={styles.panelHeader}>
      <Text size="1" weight="bold" color="gray" className={styles.sectionLabel}>
        ACTIVITY
      </Text>
    </div>
    <div className={styles.scrollList}>
      {activities.length === 0 && (
        <Text size="1" className={styles.emptyText}>
          No recent activity
        </Text>
      )}
      {activities.map((a, i) => {
        const tooltip = a.description || a.title;
        return (
          <Tooltip
            key={`${a.activity_type}-${a.timestamp}-${i}`}
            content={tooltip}
            delayDuration={150}
          >
            <div
              className={styles.listRow}
              tabIndex={0}
              role="listitem"
              aria-label={tooltip}
            >
              <span className={styles.listIcon}>{a.icon}</span>
              <div className={styles.listContent}>
                <span className={styles.listTitle}>{a.title}</span>
              </div>
              <span className={styles.listMeta}>{formatTime(a.timestamp)}</span>
            </div>
          </Tooltip>
        );
      })}
    </div>
  </div>
);
