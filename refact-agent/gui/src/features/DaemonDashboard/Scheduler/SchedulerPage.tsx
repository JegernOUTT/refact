import { useCallback, useEffect, useMemo, useState } from "react";

import { Badge, Button, FieldError } from "../../../components/ui";
import { selectConfig } from "../../Config/configSlice";
import { useAppSelector } from "../../../hooks";
import {
  resolveDaemonBaseUrl,
  useGetCronStatusQuery,
  useGetDaemonInfoQuery,
  useRestartProjectMutation,
} from "../../../services/refact/daemon";
import { isReadyWorker } from "../Projects/projectRagStatus";
import { CreateJobDialog } from "./CreateJobDialog";
import { CrossProjectJobList } from "./CrossProjectJobList";
import {
  deleteProjectCron,
  fetchCrossProjectCron,
  fetchProjectCronTasks,
  formatRelativeMs,
  runProjectCron,
  setProjectCronEnabled,
  type ProjectCronGroup,
} from "./schedulerFanout";
import styles from "./Scheduler.module.css";

const DAEMON_POLLING_INTERVAL_MS = 5_000;

function clockLabel(status: {
  enabled: boolean;
  jobs: number;
  next_wake_ms: number | null;
}): string {
  const state = status.enabled ? "on" : "off";
  const jobs = `${String(status.jobs)} ${status.jobs === 1 ? "job" : "jobs"}`;
  const wake =
    status.next_wake_ms !== null && status.next_wake_ms > 0
      ? `next wake ${formatRelativeMs(status.next_wake_ms)}`
      : "no wake scheduled";
  return `Clock ${state} · ${jobs} · ${wake}`;
}

export function SchedulerPage() {
  const config = useAppSelector(selectConfig);
  const daemonBase = resolveDaemonBaseUrl(config);
  const { data: daemonInfo } = useGetDaemonInfoQuery(undefined, {
    pollingInterval: DAEMON_POLLING_INTERVAL_MS,
  });
  const { data: cronStatus } = useGetCronStatusQuery(undefined, {
    pollingInterval: DAEMON_POLLING_INTERVAL_MS,
  });
  const [restartProject] = useRestartProjectMutation();

  const workers = useMemo(() => daemonInfo?.workers ?? [], [daemonInfo]);
  const cronPending = daemonInfo?.status.cron_pending ?? {};
  const stoppedWorkers = workers.filter((worker) => !isReadyWorker(worker));

  const [groups, setGroups] = useState<ProjectCronGroup[]>([]);
  const [loading, setLoading] = useState(true);
  const [hadErrors, setHadErrors] = useState(false);
  const [busyTaskId, setBusyTaskId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [wakingProjectId, setWakingProjectId] = useState<string | null>(null);

  const hasInfo = daemonInfo !== undefined;

  useEffect(() => {
    if (!hasInfo) return;
    const controller = new AbortController();
    setLoading(true);
    void fetchCrossProjectCron(daemonBase, workers, controller.signal)
      .then((result) => {
        if (controller.signal.aborted) return;
        setGroups(result.groups);
        setHadErrors(result.hadErrors);
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [daemonBase, hasInfo, workers]);

  const refreshProject = useCallback(
    async (projectId: string) => {
      try {
        const tasks = await fetchProjectCronTasks(daemonBase, projectId);
        setGroups((current) =>
          current.map((group) =>
            group.projectId === projectId
              ? { ...group, tasks, error: false }
              : group,
          ),
        );
      } catch {
        setGroups((current) =>
          current.map((group) =>
            group.projectId === projectId ? { ...group, error: true } : group,
          ),
        );
      }
    },
    [daemonBase],
  );

  async function withTaskAction(
    projectId: string,
    taskId: string,
    action: () => Promise<void>,
  ) {
    setBusyTaskId(taskId);
    setActionError(null);
    try {
      await action();
    } catch (error) {
      setActionError(
        error instanceof Error ? error.message : "Scheduler request failed",
      );
    } finally {
      await refreshProject(projectId);
      setBusyTaskId(null);
    }
  }

  const handleRunNow = (projectId: string, id: string) =>
    void withTaskAction(projectId, id, () =>
      runProjectCron(daemonBase, projectId, id),
    );

  const handleToggleEnabled = (
    projectId: string,
    id: string,
    enabled: boolean,
  ) =>
    void withTaskAction(projectId, id, () =>
      setProjectCronEnabled(daemonBase, projectId, id, enabled),
    );

  const handleDelete = (projectId: string, id: string) =>
    void withTaskAction(projectId, id, () =>
      deleteProjectCron(daemonBase, projectId, id),
    );

  async function handleWake(projectId: string) {
    setWakingProjectId(projectId);
    setActionError(null);
    try {
      await restartProject(projectId).unwrap();
    } catch {
      setActionError("Could not wake the project worker.");
    } finally {
      setWakingProjectId(null);
    }
  }

  const taskCounts = Object.fromEntries(
    groups.map((group) => [group.projectId, group.tasks.length]),
  );

  return (
    <section aria-labelledby="scheduler-heading" className={styles.page}>
      <header className={styles.pageHeader}>
        <div>
          <h2 id="scheduler-heading">Scheduler</h2>
          <p>Scheduled jobs across every project the daemon knows about.</p>
        </div>
        <div className={styles.headerSide}>
          {cronStatus ? (
            <Badge
              data-testid="scheduler-clock"
              tone={cronStatus.enabled ? "success" : "muted"}
              variant="soft"
            >
              {clockLabel(cronStatus)}
            </Badge>
          ) : null}
          <Button
            disabled={workers.length === 0}
            onClick={() => setCreateOpen(true)}
            variant="primary"
          >
            New job
          </Button>
        </div>
      </header>

      {hadErrors ? (
        <p className={styles.muted}>
          Some projects did not answer; their jobs may be missing below.
        </p>
      ) : null}
      {actionError ? <FieldError>{actionError}</FieldError> : null}

      <CrossProjectJobList
        busyTaskId={busyTaskId}
        cronPending={cronPending}
        groups={groups}
        loading={loading}
        onDelete={handleDelete}
        onRunNow={handleRunNow}
        onToggleEnabled={handleToggleEnabled}
        onWake={(projectId) => void handleWake(projectId)}
        stoppedWorkers={stoppedWorkers}
        wakingProjectId={wakingProjectId}
      />

      <CreateJobDialog
        daemonBase={daemonBase}
        onCreated={(projectId) => void refreshProject(projectId)}
        onOpenChange={setCreateOpen}
        onWake={(projectId) => void handleWake(projectId)}
        open={createOpen}
        taskCounts={taskCounts}
        wakingProjectId={wakingProjectId}
        workers={workers}
      />
    </section>
  );
}
