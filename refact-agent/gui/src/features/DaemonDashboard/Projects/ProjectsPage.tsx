import { useEffect, useMemo, useState } from "react";
import { FolderKanban, Plus } from "lucide-react";

import {
  Button,
  EmptyState,
  ErrorState,
  LoadingState,
} from "../../../components/ui";
import { selectConfig } from "../../Config/configSlice";
import { useAppSelector } from "../../../hooks";
import {
  resolveDaemonBaseUrl,
  useListProjectsQuery,
  type DaemonProjectOpenResponse,
  type DaemonWorker,
} from "../../../services/refact/daemon";
import { AddProjectDialog } from "./AddProjectDialog";
import { ProjectCard } from "./ProjectCard";
import {
  fetchReadyProjectStatuses,
  isReadyWorker,
  type ProjectRagStatus,
} from "./projectRagStatus";
import styles from "./Projects.module.css";

const WORKERS_POLLING_INTERVAL_MS = 4_000;

function pendingWorker(root: string): DaemonWorker {
  const normalized = root.replace(/[\\/]+$/, "");
  const slug = normalized.split(/[\\/]/).at(-1) ?? root;
  return {
    project_id: `pending:${root}`,
    slug,
    root,
    pinned: false,
    last_active_ms: null,
    state: "starting",
    pid: null,
    http_port: null,
    lsp_port: null,
    lsp_clients: 0,
    busy_chats: 0,
    exec_running: 0,
    live_proxy_streams: 0,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: null,
    last_error: null,
  };
}

function openedPendingWorker(
  response: DaemonProjectOpenResponse,
): DaemonWorker {
  return {
    ...pendingWorker(response.root),
    project_id: response.project_id,
    slug: response.slug,
    pinned: response.pinned,
  };
}

export function ProjectsPage() {
  const config = useAppSelector(selectConfig);
  const daemonBase = resolveDaemonBaseUrl(config);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [optimisticWorker, setOptimisticWorker] = useState<DaemonWorker | null>(
    null,
  );
  const [ragStatuses, setRagStatuses] = useState<
    Partial<Record<string, ProjectRagStatus>>
  >({});
  const { data, error, fulfilledTimeStamp, isLoading, refetch } =
    useListProjectsQuery(undefined, {
      pollingInterval: WORKERS_POLLING_INTERVAL_MS,
    });
  const workers = useMemo(() => data ?? [], [data]);

  useEffect(() => {
    if (
      optimisticWorker &&
      workers.some(
        (worker) =>
          worker.project_id === optimisticWorker.project_id ||
          worker.root === optimisticWorker.root,
      )
    ) {
      setOptimisticWorker(null);
    }
  }, [optimisticWorker, workers]);

  useEffect(() => {
    const readyWorkers = workers.filter(isReadyWorker);
    let active = true;

    setRagStatuses((current) =>
      Object.fromEntries(
        readyWorkers.map((worker) => [
          worker.project_id,
          current[worker.project_id] ?? { state: "loading" },
        ]),
      ),
    );

    void fetchReadyProjectStatuses(daemonBase, readyWorkers).then(
      (statuses) => {
        if (active) setRagStatuses(statuses);
      },
    );

    return () => {
      active = false;
    };
  }, [daemonBase, fulfilledTimeStamp, workers]);

  const displayedWorkers = useMemo(() => {
    if (
      !optimisticWorker ||
      workers.some(
        (worker) =>
          worker.project_id === optimisticWorker.project_id ||
          worker.root === optimisticWorker.root,
      )
    ) {
      return workers;
    }
    return [optimisticWorker, ...workers];
  }, [optimisticWorker, workers]);

  if (isLoading && displayedWorkers.length === 0) {
    return <LoadingState label="Loading projects" variant="full" />;
  }

  if (error && displayedWorkers.length === 0) {
    return (
      <ErrorState
        title="Projects are unavailable"
        error={error}
        retry={
          <Button onClick={() => void refetch()} variant="soft">
            Retry
          </Button>
        }
        variant="full"
      />
    );
  }

  return (
    <section className={styles.page} aria-labelledby="projects-heading">
      <header className={styles.pageHeader}>
        <div>
          <h2 id="projects-heading" className={styles.pageTitle}>
            Projects
          </h2>
          <p className={styles.pageDescription}>
            Open workspaces and watch their workers and indexes.
          </p>
        </div>
        {displayedWorkers.length > 0 ? (
          <Button
            leftIcon={Plus}
            onClick={() => setDialogOpen(true)}
            variant="primary"
          >
            Add project
          </Button>
        ) : null}
      </header>

      {error ? (
        <ErrorState
          className={styles.inlineError}
          title="Project refresh failed"
          error={error}
          retry={
            <Button onClick={() => void refetch()} size="sm" variant="soft">
              Retry
            </Button>
          }
        />
      ) : null}

      {displayedWorkers.length === 0 ? (
        <EmptyState
          action={
            <Button
              leftIcon={Plus}
              onClick={() => setDialogOpen(true)}
              size="lg"
              variant="primary"
            >
              Add project
            </Button>
          }
          description="Choose a local folder to start its worker and indexing."
          icon={FolderKanban}
          title="Add your first project"
          variant="full"
        />
      ) : (
        <div className={styles.grid}>
          {displayedWorkers.map((worker) => (
            <ProjectCard
              daemonBase={daemonBase}
              key={worker.project_id}
              onMutated={() => void refetch()}
              ragStatus={ragStatuses[worker.project_id]}
              worker={worker}
            />
          ))}
        </div>
      )}

      <AddProjectDialog
        onFailed={() => setOptimisticWorker(null)}
        onOpenChange={setDialogOpen}
        onOpening={(root) => setOptimisticWorker(pendingWorker(root))}
        onProjectOpened={(response) => {
          setOptimisticWorker(openedPendingWorker(response));
          void refetch();
        }}
        open={dialogOpen}
      />
    </section>
  );
}
