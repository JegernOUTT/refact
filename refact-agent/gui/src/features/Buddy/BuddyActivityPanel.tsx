import React from "react";
import { History } from "lucide-react";
import {
  Badge,
  Icon,
  SegmentedControl,
  Surface,
  Tooltip,
  Text,
} from "../../components/ui";
import type { BuddyActivityEntry } from "./types";
import { formatBuddyTime, formatFailureLabel } from "./buddyUtils";
import { BuddySectionHeader } from "./BuddySectionHeader";
import { activityIcon } from "./buddyIcons";
import styles from "./BuddyActivityPanel.module.css";

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
  const filteredActivities = React.useMemo(
    () =>
      activities.filter((entry) =>
        filter === "all" ? true : entry.activity_type.startsWith(filter),
      ),
    [activities, filter],
  );

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
      <div className={`${styles.scrollList} rf-stagger`}>
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
                className={`${styles.listRow} rf-enter-rise`}
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
                <span className={styles.listIcon}>
                  <Icon
                    icon={activityIcon(a)}
                    size="sm"
                    tone={activityTone(a)}
                  />
                </span>
                <div className={styles.listContent}>
                  <span className={styles.listTitle}>
                    {a.title}
                    {failureLabel && (
                      <Badge tone="warning">{failureLabel}</Badge>
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
