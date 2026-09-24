import React from "react";
import { History } from "lucide-react";
import {
  Badge,
  Button,
  Icon,
  SegmentedControl,
  Surface,
} from "../../components/ui";
import type { BuddyActivityEntry } from "./types";
import { formatBuddyTime, formatFailureLabel } from "./buddyUtils";
import { BuddySectionHeader } from "./BuddySectionHeader";
import { activityIcon } from "./buddyIcons";
import styles from "./BuddyActivityPanel.module.css";
import { collapseConsecutive, HISTORY_PAGE_SIZE } from "./buddyHistory";

type ActivityFilter = "all" | "refact_" | "buddy_";

interface BuddyActivityPanelProps {
  activities: BuddyActivityEntry[];
  onOpenChat?: (chatId: string, title: string) => void;
}

function activityTone(
  entry: BuddyActivityEntry,
): React.ComponentProps<typeof Icon>["tone"] {
  if (entry.failure_category) return "warning";
  if (entry.activity_type.startsWith("buddy_")) return "accent";
  return "muted";
}

export const BuddyActivityPanel: React.FC<BuddyActivityPanelProps> = ({
  activities,
  onOpenChat,
}) => {
  const [filter, setFilter] = React.useState<ActivityFilter>("all");
  const [visibleCount, setVisibleCount] = React.useState(HISTORY_PAGE_SIZE);
  const filteredActivities = React.useMemo(() => {
    const filtered = activities.filter((entry) =>
      filter === "all" ? true : entry.activity_type.startsWith(filter),
    );
    return collapseConsecutive(
      filtered,
      (entry) => `${entry.activity_type}\u0000${entry.title}`,
    );
  }, [activities, filter]);
  const visibleActivities = filteredActivities.slice(0, visibleCount);

  return (
    <Surface
      className={styles.panel}
      data-testid="buddy-activity-panel"
      animated="rise"
      radius="card"
      variant="glass"
    >
      <BuddySectionHeader icon={History} label="Activity" />
      <SegmentedControl
        aria-label="activity filter"
        className={styles.filter}
        name="buddy-activity-filter"
        size="sm"
        value={filter}
        onValueChange={(value) => setFilter(value as ActivityFilter)}
        options={[
          { value: "all", label: "All" },
          { value: "refact_", label: "refact_*" },
          { value: "buddy_", label: "buddy_*" },
        ]}
      />
      <ul className={`${styles.scrollList} rf-stagger`}>
        {filteredActivities.length === 0 && (
          <li className={styles.emptyText}>No recent activity</li>
        )}
        {visibleActivities.map(({ entry, count }) => {
          const failureLabel = formatFailureLabel(entry.failure_category);
          const detail = entry.failure_summary ?? entry.description;
          const canOpen = Boolean(entry.chat_id && onOpenChat);
          const content = (
            <>
              <span className={styles.listIcon}>
                <Icon
                  icon={activityIcon(entry)}
                  size="sm"
                  tone={activityTone(entry)}
                />
              </span>
              <span className={styles.listContent}>
                <span className={styles.listTitle}>
                  {entry.title}
                  {count > 1 && <Badge tone="muted">{count} similar</Badge>}
                  {failureLabel && <Badge tone="warning">{failureLabel}</Badge>}
                </span>
                {detail && (
                  <span className={styles.listSubtitle}>{detail}</span>
                )}
              </span>
              <span className={styles.listMeta}>
                {formatBuddyTime(entry.timestamp)}
              </span>
            </>
          );
          return (
            <li
              key={`${entry.activity_type}-${entry.timestamp}-${entry.title}`}
              className={styles.listItem}
            >
              {canOpen ? (
                <button
                  type="button"
                  className={`${styles.listRow} rf-enter-rise`}
                  data-clickable="true"
                  aria-label={`${entry.title}.${
                    failureLabel ? ` ${failureLabel}.` : ""
                  }${detail ? ` ${detail}.` : ""} Open Buddy chat`}
                  onClick={() => {
                    if (entry.chat_id) onOpenChat?.(entry.chat_id, entry.title);
                  }}
                >
                  {content}
                </button>
              ) : (
                <div className={`${styles.listRow} rf-enter-rise`}>
                  {content}
                </div>
              )}
            </li>
          );
        })}
      </ul>
      {visibleActivities.length < filteredActivities.length && (
        <Button
          type="button"
          variant="ghost"
          onClick={() => setVisibleCount((count) => count + HISTORY_PAGE_SIZE)}
        >
          Show more
        </Button>
      )}
    </Surface>
  );
};
