import React from "react";
import {
  Badge,
  Button,
  EmptyState,
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

function formatNextFire(timestampMs: number): string {
  if (timestampMs <= 0) return "—";
  return new Date(timestampMs).toLocaleString();
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
        title="No scheduled prompts yet."
        description="Create a cron prompt above to wake this chat on a schedule."
      />
    );
  }

  return (
    <div className={styles.jobList}>
      {tasks.map((task) => (
        <Surface
          animated="rise"
          as="article"
          className={styles.jobCard}
          key={task.id}
          variant="surface-1"
        >
          <div className={styles.jobHeader}>
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
            <div className={styles.jobMetaItem}>
              <dt>Next fire</dt>
              <dd>{formatNextFire(task.next_fire_at_ms)}</dd>
            </div>
            <div className={styles.jobMetaItem}>
              <dt>Fire count</dt>
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
      ))}
    </div>
  );
};
