import React from "react";
import { Badge, Button, Spinner, Text } from "@radix-ui/themes";
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
    return (
      <div className={styles.empty}>
        <Spinner />
      </div>
    );
  }

  if (tasks.length === 0) {
    return <div className={styles.empty}>No scheduled prompts yet.</div>;
  }

  return (
    <div className={styles.tableWrap}>
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
                <Text weight="medium">{task.human_schedule}</Text>
                <br />
                <Text size="1" color="gray">
                  {task.cron}
                </Text>
              </td>
              <td>{formatNextFire(task.next_fire_at_ms)}</td>
              <td>{task.fire_count}</td>
              <td>
                <Badge color={task.durable ? "blue" : "gray"}>
                  {task.durable ? "Durable" : "Session"}
                </Badge>
              </td>
              <td>
                <Badge color={task.recurring ? "green" : "orange"}>
                  {task.recurring ? "Recurring" : "One-shot"}
                </Badge>
              </td>
              <td>{task.description}</td>
              <td className={styles.actions}>
                <Button
                  color="red"
                  variant="soft"
                  size="1"
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
