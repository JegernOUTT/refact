import React from "react";
import { SegmentedControl, Text, Tooltip } from "@radix-ui/themes";
import classNames from "classnames";
import type { BuddyActivityEntry } from "./types";
import styles from "./BuddyHome.module.css";

type ActivityFilter = "all" | "refact_" | "buddy_";

function formatTime(ts: string): string {
  if (!ts) return "";
  return new Date(ts).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

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
    <div
      className={classNames(styles.panel, styles.panelScroll)}
      data-testid="buddy-activity-panel"
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
      <SegmentedControl.Root
        size="1"
        value={filter}
        onValueChange={(value) => setFilter(value as ActivityFilter)}
      >
        <SegmentedControl.Item value="all">All</SegmentedControl.Item>
        <SegmentedControl.Item value="refact_">refact_*</SegmentedControl.Item>
        <SegmentedControl.Item value="buddy_">buddy_*</SegmentedControl.Item>
      </SegmentedControl.Root>
      <div className={styles.scrollList}>
        {filteredActivities.length === 0 && (
          <Text size="1" className={styles.emptyText}>
            No recent activity
          </Text>
        )}
        {filteredActivities.map((a, i) => {
          const tooltip = a.description || a.title;
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
                tabIndex={0}
                role={canOpen ? "button" : "listitem"}
                aria-label={canOpen ? `${tooltip}. Open Buddy chat` : tooltip}
                onClick={() => {
                  if (a.chat_id) onOpenChat?.(a.chat_id, a.title);
                }}
                onKeyDown={(event) => {
                  if (!a.chat_id || !onOpenChat) return;
                  if (event.key !== "Enter" && event.key !== " ") return;
                  event.preventDefault();
                  onOpenChat(a.chat_id, a.title);
                }}
              >
                <span className={styles.listIcon}>{a.icon}</span>
                <div className={styles.listContent}>
                  <span className={styles.listTitle}>{a.title}</span>
                </div>
                <span className={styles.listMeta}>
                  {formatTime(a.timestamp)}
                </span>
              </div>
            </Tooltip>
          );
        })}
      </div>
    </div>
  );
};
