import React from "react";
import { History } from "lucide-react";
import { Button, SegmentedControl } from "../../components/ui";
import type { BuddyActivityEntry } from "./types";
import {
  BUDDY_VERBS,
  BuddyEmptyState,
  BuddyRow,
  BuddySection,
  BuddyTimestamp,
  buddyFailureLabel,
  useBuddyPersistentState,
} from "./ui";
import { activityIcon } from "./buddyIcons";
import styles from "./BuddyActivityPanel.module.css";
import { collapseConsecutive, HISTORY_PAGE_SIZE } from "./buddyHistory";

type ActivityFilter = "all" | "refact_" | "buddy_";

const FILTER_OPTIONS: { value: ActivityFilter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "refact_", label: "Agent checks" },
  { value: "buddy_", label: "Buddy workflows" },
];

function isActivityFilter(value: string): value is ActivityFilter {
  return FILTER_OPTIONS.some((option) => option.value === value);
}

interface BuddyActivityPanelProps {
  activities: BuddyActivityEntry[];
  onOpenChat?: (chatId: string, title: string) => void;
}

export const BuddyActivityPanel: React.FC<BuddyActivityPanelProps> = ({
  activities,
  onOpenChat,
}) => {
  const [filter, setFilter] = useBuddyPersistentState<ActivityFilter>(
    "buddy.activity.legacyFilter",
    "all",
  );
  const [visibleCount, setVisibleCount] = useBuddyPersistentState(
    "buddy.activity.legacyVisible",
    HISTORY_PAGE_SIZE,
  );
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
    <BuddySection
      title="Activity"
      icon={History}
      count={filteredActivities.length}
      testId="buddy-activity-panel"
      footer={
        visibleActivities.length < filteredActivities.length ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() =>
              setVisibleCount((count) => count + HISTORY_PAGE_SIZE)
            }
          >
            {BUDDY_VERBS.showMore}
          </Button>
        ) : null
      }
    >
      <SegmentedControl
        aria-label="Activity filter"
        className={styles.filter}
        name="buddy-activity-filter"
        size="md"
        value={filter}
        onValueChange={(value) => {
          if (isActivityFilter(value)) setFilter(value);
        }}
        options={FILTER_OPTIONS}
      />
      {filteredActivities.length === 0 ? (
        <BuddyEmptyState compact icon={History} title="No recent activity" />
      ) : (
        <ul className={styles.list}>
          {visibleActivities.map(({ entry, count }) => {
            const failureLabel = entry.failure_category
              ? buddyFailureLabel(entry.failure_category)
              : null;
            const detail = entry.failure_summary ?? entry.description;
            const chatId = entry.chat_id;
            const canOpen = Boolean(chatId && onOpenChat);
            return (
              <li
                key={`${entry.activity_type}-${entry.timestamp}-${entry.title}`}
              >
                <BuddyRow
                  title={entry.title}
                  description={detail || undefined}
                  leading={{
                    icon: activityIcon(entry),
                    tone: entry.failure_category ? "warning" : "muted",
                  }}
                  meta={[
                    failureLabel,
                    count > 1 ? `${count} similar` : null,
                    <BuddyTimestamp key="time" value={entry.timestamp} />,
                  ]}
                  onClick={
                    canOpen && chatId
                      ? () => onOpenChat?.(chatId, entry.title)
                      : undefined
                  }
                  aria-label={
                    canOpen
                      ? `${entry.title}.${
                          failureLabel ? ` ${failureLabel}.` : ""
                        }${detail ? ` ${detail}.` : ""} Open Buddy chat`
                      : undefined
                  }
                />
              </li>
            );
          })}
        </ul>
      )}
    </BuddySection>
  );
};
