import React from "react";
import { CalendarClock, Timer } from "lucide-react";
import {
  Badge,
  Button,
  EmptyState,
  Icon,
  LoadingState,
  Surface,
} from "../../components/ui";
import type { CronTask } from "../../services/refact/schedulerApi";
import styles from "./Scheduler.module.css";

type CronListProps = {
  tasks: CronTask[];
  isLoading?: boolean;
  deletingId?: string | null;
  onDelete: (id: string) => void;
};

type NextFireDisplay = {
  primary: string;
  absolute?: string;
  title?: string;
};

function formatDuration(ms: number): string {
  const minutes = Math.max(1, Math.round(ms / 60000));
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor((minutes % 1440) / 60);
  const mins = minutes % 60;

  if (days > 0) {
    return `in ${days}d${hours > 0 ? ` ${hours}h` : ""}`;
  }
  if (hours > 0) {
    return `in ${hours}h${mins > 0 ? ` ${mins}m` : ""}`;
  }
  return `in ${mins}m`;
}

function formatNextFire(timestampMs: number): NextFireDisplay {
  if (timestampMs <= 0) return { primary: "—" };

  const absolute = new Date(timestampMs).toLocaleString();
  const remaining = timestampMs - Date.now();
  if (remaining > 0) {
    return {
      primary: formatDuration(remaining),
      absolute,
      title: absolute,
    };
  }

  return { primary: absolute };
}

export const CronList: React.FC<CronListProps> = ({
  tasks,
  isLoading = false,
  deletingId = null,
  onDelete,
}) => {
  if (isLoading) {
    return <LoadingState label="Loading scheduled prompts" />;
  }

  if (tasks.length === 0) {
    return (
      <EmptyState
        className={styles.emptyState}
        icon={CalendarClock}
        title="No scheduled prompts yet."
        description="Create a cron prompt to wake this chat on a schedule."
      />
    );
  }

  return (
    <div className={styles.jobList}>
      {tasks.map((task) => {
        const nextFire = formatNextFire(task.next_fire_at_ms);

        return (
          <Surface
            animated="rise"
            as="article"
            className={styles.jobCard}
            key={task.id}
            variant="surface-1"
          >
            <div className={styles.jobHeader}>
              <div className={styles.iconTile} aria-hidden="true">
                <Icon icon={Timer} tone="accent" />
              </div>
              <div className={styles.jobTitleBlock}>
                <h3 className={styles.jobTitle}>{task.human_schedule}</h3>
                <code className={styles.jobCron}>{task.cron}</code>
              </div>
              <div className={styles.jobBadges}>
                <Badge tone={task.durable ? "accent" : "muted"}>
                  {task.durable ? "Durable" : "Session"}
                </Badge>
                <Badge tone={task.recurring ? "success" : "warning"}>
                  {task.recurring ? "Recurring" : "One-shot"}
                </Badge>
              </div>
            </div>

            <p className={styles.jobDescription}>{task.description}</p>

            <dl className={styles.jobMeta}>
              <div
                className={styles.jobMetaItem}
                title={nextFire.title ?? undefined}
              >
                <dt>Next fire</dt>
                <dd>
                  <span>{nextFire.primary}</span>
                  {nextFire.absolute ? (
                    <span className={styles.metaSecondary}>
                      {nextFire.absolute}
                    </span>
                  ) : null}
                </dd>
              </div>
              <div className={styles.jobMetaItem}>
                <dt>Fires</dt>
                <dd>{task.fire_count}</dd>
              </div>
            </dl>

            <div className={styles.jobActions}>
              <Button
                variant="danger"
                size="sm"
                disabled={deletingId === task.id}
                onClick={() => onDelete(task.id)}
              >
                Delete
              </Button>
            </div>
          </Surface>
        );
      })}
    </div>
  );
};
