import React, { useMemo } from "react";
import { Text } from "@radix-ui/themes";
import { CheckboxIcon } from "@radix-ui/react-icons";
import type { TodoItem, TodoStatus } from "../../../Chat/Thread/types";
import type { DashboardBreakpoint } from "../../types";
import styles from "./TodoProgress.module.css";

type TodoProgressProps = {
  todos: TodoItem[];
  breakpoint: DashboardBreakpoint;
};

const STATUS_PRIORITY: Record<TodoStatus, number> = {
  in_progress: 0,
  failed: 1,
  pending: 2,
  completed: 3,
};

export const TodoProgress: React.FC<TodoProgressProps> = ({
  todos,
  breakpoint,
}) => {
  const sorted = useMemo(() => {
    return [...todos].sort((a, b) => {
      return STATUS_PRIORITY[a.status] - STATUS_PRIORITY[b.status];
    });
  }, [todos]);

  if (todos.length === 0) return null;

  const done = todos.filter((t) => t.status === "completed").length;
  const total = todos.length;

  if (breakpoint === "narrow") {
    return (
      <div className={styles.compact}>
        <Text size="1" color="gray">
          <CheckboxIcon /> {done}/{total}
        </Text>
        <div className={styles.miniBar}>
          {sorted.slice(0, 12).map((t) => (
            <div
              key={t.id}
              className={styles.miniSegment}
              data-status={t.status}
            />
          ))}
        </div>
      </div>
    );
  }

  const MAX_VISIBLE = 3;
  const visible = sorted.slice(0, MAX_VISIBLE);
  const remaining = todos.length - MAX_VISIBLE;

  return (
    <div className={styles.list}>
      {visible.map((t) => (
        <div key={t.id} className={styles.item}>
          <span className={styles.statusDot} data-status={t.status} />
          <Text size="1" truncate className={styles.itemText}>
            {t.content}
          </Text>
        </div>
      ))}
      {remaining > 0 && (
        <Text size="1" color="gray">
          +{remaining} more
        </Text>
      )}
    </div>
  );
};
