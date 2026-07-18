import { Button, Surface } from "../../../../../components/ui";
import type { DaemonWorker } from "../../../../../services/refact/daemon";
import type { TaskMeta } from "../../../../../services/refact/tasks";
import { useProjectResource } from "../projectResource";
import { WorkerGate } from "../WorkerGate";
import { Fact, ResourceView } from "./shared";
import styles from "../ProjectDetail.module.css";

const MAX_ACTIVE_TASKS = 5;

const TASK_COLUMNS = [
  "planning",
  "active",
  "paused",
  "completed",
  "abandoned",
] as const;

type TasksTabProps = {
  daemonBase: string;
  worker: DaemonWorker;
  openUrl: string;
  onMutated: () => void;
};

function parseTasks(data: unknown): TaskMeta[] | null {
  return Array.isArray(data) ? (data as TaskMeta[]) : null;
}

function taskCountsByStatus(
  tasks: TaskMeta[],
): Record<(typeof TASK_COLUMNS)[number], number> {
  const counts = {
    planning: 0,
    active: 0,
    paused: 0,
    completed: 0,
    abandoned: 0,
  };
  for (const task of tasks) {
    if (task.status in counts) counts[task.status] += 1;
  }
  return counts;
}

function topActiveTasks(tasks: TaskMeta[], limit: number): TaskMeta[] {
  return tasks
    .filter((task) => task.status === "active")
    .sort(
      (left, right) =>
        Date.parse(right.updated_at) - Date.parse(left.updated_at),
    )
    .slice(0, limit);
}

function TasksContent({
  daemonBase,
  projectId,
  openUrl,
}: {
  daemonBase: string;
  projectId: string;
  openUrl: string;
}) {
  const tasks = useProjectResource(daemonBase, projectId, "/tasks", parseTasks);

  return (
    <Surface className={styles.section} radius="card" variant="glass">
      <h3 className={styles.sectionTitle}>Task board</h3>
      <ResourceView
        errorText="Tasks are unavailable."
        resource={tasks.resource}
      >
        {(items) => {
          const counts = taskCountsByStatus(items);
          const activeTasks = topActiveTasks(items, MAX_ACTIVE_TASKS);
          return (
            <>
              <dl className={styles.factGrid}>
                {TASK_COLUMNS.map((column) => (
                  <Fact
                    key={column}
                    label={column.charAt(0).toUpperCase() + column.slice(1)}
                    value={counts[column]}
                  />
                ))}
              </dl>
              {activeTasks.length === 0 ? (
                <p className={styles.muted}>No active tasks.</p>
              ) : (
                <ul aria-label="Active tasks" className={styles.list}>
                  {activeTasks.map((task) => (
                    <li className={styles.row} key={task.id}>
                      <span className={styles.rowCopy}>
                        <strong>{task.name}</strong>
                        <span>
                          {task.cards_done}/{task.cards_total} cards ·{" "}
                          {task.agents_active} agents
                        </span>
                      </span>
                      <span className={styles.rowMeta}>{task.status}</span>
                    </li>
                  ))}
                </ul>
              )}
              <div className={styles.actions}>
                <Button asChild size="sm" variant="soft">
                  <a href={openUrl}>Open board</a>
                </Button>
              </div>
            </>
          );
        }}
      </ResourceView>
    </Surface>
  );
}

export function TasksTab({
  daemonBase,
  worker,
  openUrl,
  onMutated,
}: TasksTabProps) {
  return (
    <div className={styles.tabBody}>
      <WorkerGate onMutated={onMutated} worker={worker}>
        <TasksContent
          daemonBase={daemonBase}
          openUrl={openUrl}
          projectId={worker.project_id}
        />
      </WorkerGate>
    </div>
  );
}
