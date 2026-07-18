import { useState } from "react";
import { CalendarClock, ChevronDown, ChevronRight } from "lucide-react";

import {
  Badge,
  Button,
  EmptyState,
  Icon,
  LoadingState,
  Surface,
} from "../../../components/ui";
import type { CronTask } from "../../../services/refact/schedulerApi";
import type { DaemonWorker } from "../../../services/refact/daemon";
import { formatRelativeMs, type ProjectCronGroup } from "./schedulerFanout";
import styles from "./Scheduler.module.css";

type CrossProjectJobListProps = {
  groups: ProjectCronGroup[];
  stoppedWorkers: DaemonWorker[];
  cronPending: Record<string, number>;
  loading: boolean;
  busyTaskId: string | null;
  wakingProjectId: string | null;
  onRunNow: (projectId: string, id: string) => void;
  onToggleEnabled: (projectId: string, id: string, enabled: boolean) => void;
  onDelete: (projectId: string, id: string) => void;
  onWake: (projectId: string) => void;
};

function nextFireLabel(task: CronTask): string {
  if (task.next_fire_at_ms <= 0) return "—";
  return formatRelativeMs(task.next_fire_at_ms);
}

function lastRunStatus(task: CronTask): string {
  const lastRun =
    task.recent_runs.length > 0
      ? task.recent_runs[task.recent_runs.length - 1]
      : null;
  return task.last_status ?? lastRun?.status ?? "Pending";
}

function lastRunTone(task: CronTask): "success" | "danger" | "muted" {
  const status = lastRunStatus(task).toLowerCase();
  if (["fired", "ok", "success", "completed"].includes(status)) {
    return "success";
  }
  if (status.includes("error") || status.includes("fail")) return "danger";
  return "muted";
}

export function CrossProjectJobList({
  groups,
  stoppedWorkers,
  cronPending,
  loading,
  busyTaskId,
  wakingProjectId,
  onRunNow,
  onToggleEnabled,
  onDelete,
  onWake,
}: CrossProjectJobListProps) {
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});

  if (loading && groups.length === 0) {
    return <LoadingState label="Loading scheduled jobs" />;
  }

  if (groups.length === 0 && stoppedWorkers.length === 0) {
    return (
      <EmptyState
        icon={CalendarClock}
        title="No projects to schedule"
        description="Open a project so its scheduler becomes available."
      />
    );
  }

  const toggleGroup = (projectId: string) => {
    setCollapsed((current) => ({
      ...current,
      [projectId]: !current[projectId],
    }));
  };

  return (
    <div className={styles.groupList}>
      {groups.map((group) => {
        const isCollapsed = Boolean(collapsed[group.projectId]);
        const pendingAt = cronPending[group.projectId];
        return (
          <Surface
            animated="rise"
            as="section"
            className={styles.group}
            key={group.projectId}
            variant="surface-1"
          >
            <header className={styles.groupHeader}>
              <button
                aria-expanded={!isCollapsed}
                className={styles.groupToggle}
                onClick={() => toggleGroup(group.projectId)}
                type="button"
              >
                <Icon
                  icon={isCollapsed ? ChevronRight : ChevronDown}
                  size="sm"
                />
                <span className={styles.groupTitle}>{group.slug}</span>
              </button>
              <div className={styles.groupBadges}>
                {typeof pendingAt === "number" ? (
                  <Badge tone="accent" variant="soft">
                    daemon wake {formatRelativeMs(pendingAt)}
                  </Badge>
                ) : null}
                <Badge tone="muted" variant="soft">
                  {group.tasks.length}{" "}
                  {group.tasks.length === 1 ? "job" : "jobs"}
                </Badge>
              </div>
            </header>

            {!isCollapsed && group.error ? (
              <p className={styles.groupError} role="alert">
                Could not load this project&apos;s schedule.
              </p>
            ) : null}

            {!isCollapsed && !group.error && group.tasks.length === 0 ? (
              <p className={styles.groupEmpty}>No scheduled jobs.</p>
            ) : null}

            {!isCollapsed && group.tasks.length > 0 ? (
              <ul className={styles.jobList}>
                {group.tasks.map((task) => {
                  const busy = busyTaskId === task.id;
                  return (
                    <li className={styles.jobRow} key={task.id}>
                      <div className={styles.jobCopy}>
                        <div className={styles.jobHeadline}>
                          <strong>{task.description}</strong>
                          <code className={styles.jobSchedule}>
                            {task.human_schedule}
                          </code>
                        </div>
                        <div className={styles.jobBadges}>
                          <Badge tone={task.enabled ? "success" : "warning"}>
                            {task.enabled ? "Enabled" : "Paused"}
                          </Badge>
                          <Badge tone={lastRunTone(task)}>
                            {lastRunStatus(task)}
                          </Badge>
                          <span className={styles.jobNextFire}>
                            Next fire {nextFireLabel(task)}
                          </span>
                        </div>
                      </div>
                      <div className={styles.jobActions}>
                        <Button
                          disabled={busy}
                          onClick={() => onRunNow(group.projectId, task.id)}
                          size="sm"
                          variant="soft"
                        >
                          Run now
                        </Button>
                        <Button
                          disabled={busy}
                          onClick={() =>
                            onToggleEnabled(
                              group.projectId,
                              task.id,
                              !task.enabled,
                            )
                          }
                          size="sm"
                          variant="soft"
                        >
                          {task.enabled ? "Pause" : "Resume"}
                        </Button>
                        <Button
                          disabled={busy}
                          onClick={() => onDelete(group.projectId, task.id)}
                          size="sm"
                          variant="danger"
                        >
                          Delete
                        </Button>
                      </div>
                    </li>
                  );
                })}
              </ul>
            ) : null}
          </Surface>
        );
      })}

      {stoppedWorkers.map((worker) => (
        <Surface
          animated="rise"
          as="section"
          className={styles.group}
          key={worker.project_id}
          variant="surface-1"
        >
          <header className={styles.groupHeader}>
            <div className={styles.groupToggle}>
              <span className={styles.groupTitle}>{worker.slug}</span>
              <Badge tone="muted" variant="soft">
                Stopped
              </Badge>
            </div>
            <Button
              loading={wakingProjectId === worker.project_id}
              onClick={() => onWake(worker.project_id)}
              size="sm"
              variant="soft"
            >
              Wake to view
            </Button>
          </header>
        </Surface>
      ))}
    </div>
  );
}
