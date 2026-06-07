import React from "react";
import { Badge, Button, LoadingState } from "../../components/ui";
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
    return <div className={styles.empty}>No scheduled prompts yet.</div>;
  }

  return (
    <div className={styles.scrollX}>
      <table className={styles.table}>
        <thead>
          <tr>
            <th>Schedule</th>
            <th>Next fire</th>
            <th>Fire count</th>
            <th>Scope</th>
            <th>Type</th>
            <th>Description</th>
            <th className={styles.actions}>Actions</th>
          </tr>
        </thead>
        <tbody>
          {tasks.map((task) => (
            <tr key={task.id}>
              <td>
                <div className={styles.scheduleCell}>
                  <span className={styles.scheduleTitle}>{task.human_schedule}</span>
                  <span className={styles.scheduleCron}>{task.cron}</span>
                </div>
              </td>
              <td>{formatNextFire(task.next_fire_at_ms)}</td>
              <td>{task.fire_count}</td>
              <td>
                <Badge tone={task.durable ? "accent" : "muted"}>
                  {task.durable ? "Durable" : "Session"}
                </Badge>
              </td>
              <td>
                <Badge tone={task.recurring ? "success" : "warning"}>
                  {task.recurring ? "Recurring" : "One-shot"}
                </Badge>
              </td>
              <td>{task.description}</td>
              <td className={styles.actions}>
                <Button
                  variant="danger"
                  size="sm"
                  disabled={deletingId === task.id}
                  onClick={() => onDelete(task.id)}
                >
                  Delete
                </Button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
};
