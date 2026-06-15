import React from "react";
import { Bug, CircleCheck, ShieldAlert } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import {
  Badge,
  Button,
  Icon,
  Surface,
  Text,
  Tooltip,
} from "../../components/ui";
import type { BadgeTone } from "../../components/ui";
import type { BuddyRuntimeEvent } from "./types";
import { formatBuddyTime, formatFailureLabel } from "./buddyUtils";
import { BuddySectionHeader } from "./BuddySectionHeader";
import styles from "./BuddyRecentErrorsPanel.module.css";

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

const priorityTone = (priority: RecentBuddyError["priority"]): BadgeTone => {
  if (priority === "critical") return "danger";
  if (priority === "high") return "warning";
  return "muted";
};

function errorIcon(
  event: RecentBuddyError,
  acknowledged: boolean,
): {
  icon: LucideIcon;
  tone: React.ComponentProps<typeof Icon>["tone"];
} {
  if (acknowledged) return { icon: CircleCheck, tone: "success" };
  if (event.priority === "critical")
    return { icon: ShieldAlert, tone: "danger" };
  return { icon: Bug, tone: "warning" };
}

export const BuddyRecentErrorsPanel: React.FC<BuddyRecentErrorsPanelProps> = ({
  recentErrors,
  onInvestigate,
  onDismiss,
}) => (
  <Surface
    className={styles.panel}
    data-testid="buddy-recent-errors-panel"
    animated="rise"
    radius="card"
    variant="glass"
  >
    <BuddySectionHeader icon={Bug} label="Recent errors" />
    <div className={`${styles.scrollList} rf-stagger`}>
      {recentErrors.length === 0 && (
        <Text size="1" className={styles.emptyText}>
          No errors recorded — all clear
        </Text>
      )}
      {recentErrors.map((e) => {
        const acknowledged = Boolean(e.dismissedAll ?? e.dismissed);
        const { icon, tone } = errorIcon(e, acknowledged);
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
            className={`${styles.listRow} rf-enter-rise`}
            data-acknowledged={acknowledged ? "true" : undefined}
            data-priority={e.priority}
          >
            <span className={styles.listIcon}>
              <Icon icon={icon} size="sm" tone={tone} />
            </span>
            <div className={styles.listContent}>
              <span className={styles.listTitle}>
                {e.title}
                {e.occurrences != null && e.occurrences > 1 && (
                  <Badge tone="accent">×{e.occurrences}</Badge>
                )}
                {failureLabel && <Badge tone="warning">{failureLabel}</Badge>}
                {acknowledged && <Badge tone="success">acknowledged</Badge>}
                {!acknowledged && (
                  <Badge tone={priorityTone(e.priority)}>{e.priority}</Badge>
                )}
              </span>
              {detail && <span className={styles.listSubtitle}>{detail}</span>}
              <span className={styles.listMeta}>
                {formatBuddyTime(e.created_at)}
              </span>
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
          </div>
        );
      })}
    </div>
  </Surface>
);
