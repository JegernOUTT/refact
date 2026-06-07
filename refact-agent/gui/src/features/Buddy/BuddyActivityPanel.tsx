import React from "react";
import { Text, Tooltip } from "@radix-ui/themes";
import { SegmentedControl, Surface } from "../../components/ui";
import classNames from "classnames";
import type { BuddyActivityEntry } from "./types";
import { formatBuddyTime, formatFailureLabel } from "./buddyUtils";
import styles from "./BuddyHome.module.css";

type ActivityFilter = "all" | "refact_" | "buddy_";

interface BuddyActivityPanelProps {
  activities: BuddyActivityEntry[];
  onOpenChat?: (chatId: string, title: string) => void;
}

export const BuddyActivityPanel: React.FC<BuddyActivityPanelProps> = ({
  activities,
  onOpenChat,
}) => {
  const [filter, setFilter] = React.useState<ActivityFilter>("all");
  const filteredActivities = React.useMemo(
    () =>
      activities.filter((entry) =>
        filter === "all" ? true : entry.activity_type.startsWith(filter),
      ),
    [activities, filter],
  );

  return (
    <Surface
      className={classNames(styles.panel, styles.panelScroll)}
      data-testid="buddy-activity-panel"
      radius="card"
      variant="surface-1"
    >
      <div className={styles.panelHeader}>
        <Text
          size="1"
          weight="bold"
          color="gray"
          className={styles.sectionLabel}
        >
          ACTIVITY
        </Text>
      </div>
      <SegmentedControl
        size="sm"
        value={filter}
        onValueChange={(value) => setFilter(value as ActivityFilter)}
        options={[
          { value: "all", label: "All" },
          { value: "refact_", label: "refact_*" },
          { value: "buddy_", label: "buddy_*" },
        ]}
      />
      <div className={styles.scrollList}>
        {filteredActivities.length === 0 && (
          <Text size="1" className={styles.emptyText}>
            No recent activity
          </Text>
        )}
        {filteredActivities.map((a, i) => {
          const failureLabel = formatFailureLabel(a.failure_category);
          const detail = a.failure_summary ?? a.description;
          const tooltip = detail;
          const canOpen = Boolean(a.chat_id && onOpenChat);
          return (
            <Tooltip
              key={`${a.activity_type}-${a.timestamp}-${i}`}
              content={tooltip}
              delayDuration={150}
            >
              <div
                className={styles.listRow}
                data-clickable={canOpen ? "true" : undefined}
                {...(canOpen
                  ? {
                      tabIndex: 0,
                      role: "button",
                      "aria-label": `${tooltip}. Open Buddy chat`,
                      onClick: () => {
                        if (a.chat_id) onOpenChat?.(a.chat_id, a.title);
                      },
                      onKeyDown: (
                        event: React.KeyboardEvent<HTMLDivElement>,
                      ) => {
                        if (!a.chat_id || !onOpenChat) return;
                        if (event.key !== "Enter" && event.key !== " ") return;
                        event.preventDefault();
                        onOpenChat(a.chat_id, a.title);
                      },
                    }
                  : {})}
              >
                <span className={styles.listIcon}>{a.icon}</span>
                <div className={styles.listContent}>
                  <span className={styles.listTitle}>
                    {a.title}
                    {failureLabel && (
                      <span className={styles.ackBadge}>{failureLabel}</span>
                    )}
                  </span>
                  {detail && (
                    <span className={styles.listSubtitle}>{detail}</span>
                  )}
                </div>
                <span className={styles.listMeta}>
                  {formatBuddyTime(a.timestamp)}
                </span>
              </div>
            </Tooltip>
          );
        })}
      </div>
    </Surface>
  );
};
