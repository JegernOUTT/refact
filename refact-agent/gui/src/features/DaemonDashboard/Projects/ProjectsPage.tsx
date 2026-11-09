import { useObservationPolling } from "../useObservationPolling";
import { useEffect, useMemo, useState } from "react";
import { FolderKanban, Plus } from "lucide-react";

import {
  Button,
  EmptyState,
  ErrorState,
  LoadingState,
  FieldText,
  Button as FilterButton,
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
import { MissingProjectsSection } from "./MissingProjectsSection";
import { ProjectCard } from "./ProjectCard";
import { splitProjectWorkers } from "./projectOrdering";
import { workerStateName } from "./projectRagStatus";
import styles from "./Projects.module.css";

function pendingWorker(root: string): DaemonWorker {
  const normalized = root.replace(/[\\/]+$/, "");
  const slug = normalized.split(/[\\/]/).at(-1) ?? root;
  return {
    project_id: `pending:${root}`,
    slug,
    root,
    root_exists: true,
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
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const [sort, setSort] = useState("recent");
  const { data, error, isLoading, refetch } = useListProjectsQuery(
    undefined,
    useObservationPolling(),
  );
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

  const { present: allPresent, missing } = useMemo(
    () => splitProjectWorkers(displayedWorkers),
    [displayedWorkers],
  );

  const present = allPresent
    .filter(
      (worker) =>
        `${worker.slug} ${worker.root}`
          .toLowerCase()
          .includes(search.toLowerCase()) &&
        (filter === "all" ||
          (filter === "attention"
            ? Boolean(worker.last_error) ||
              ["crashed", "failed"].includes(workerStateName(worker))
            : workerStateName(worker) === filter)),
    )
    .sort((a, b) =>
      sort === "recent"
        ? (b.last_active_ms ?? 0) - (a.last_active_ms ?? 0) ||
          a.slug.localeCompare(b.slug)
        : a.slug.localeCompare(b.slug),
    );

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
            Your workspaces, engines and local activity.
          </p>
        </div>
        {displayedWorkers.length > 0 ? (
          <Button
            className={styles.primaryAction}
            leftIcon={Plus}
            onClick={() => setDialogOpen(true)}
            variant="primary"
          >
            Add project
          </Button>
        ) : null}
      </header>

      <div className={styles.consoleMetrics}>
        <div>
          <strong>{displayedWorkers.length}</strong>
          <span>Projects</span>
        </div>
        <div>
          <strong>
            {
              displayedWorkers.filter((worker) => worker.state === "ready")
                .length
            }
          </strong>
          <span>Running workers</span>
        </div>
        <div>
          <strong>
            {displayedWorkers.reduce(
              (count, worker) => count + worker.busy_chats,
              0,
            )}
          </strong>
          <span>Busy chats reported</span>
        </div>
        <div>
          <strong>
            {
              displayedWorkers.filter((worker) => worker.state === "stopped")
                .length
            }
          </strong>
          <span>Stopped</span>
        </div>
      </div>
      <div className={styles.consoleFilters}>
        <FieldText
          aria-label="Search projects"
          placeholder="Search projects or paths…"
          value={search}
          onChange={setSearch}
        />
        {["all", "ready", "stopped", "attention"].map((value) => (
          <FilterButton
            key={value}
            size="sm"
            variant={filter === value ? "soft" : "ghost"}
            aria-pressed={filter === value}
            onClick={() => setFilter(value)}
          >
            {value === "all"
              ? "All projects"
              : value === "ready"
                ? "Running"
                : value === "stopped"
                  ? "Stopped"
                  : "Needs attention"}
          </FilterButton>
        ))}
        <FilterButton
          size="sm"
          variant="ghost"
          onClick={() => setSort(sort === "recent" ? "name" : "recent")}
          aria-label="Change project sort"
        >
          {sort === "recent" ? "Recent activity" : "Name"}
        </FilterButton>
      </div>
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
        <>
          {present.length > 0 ? (
            <div className={styles.grid}>
              {[true, false].map((pinned) => {
                const group = present.filter(
                  (worker) => worker.pinned === pinned,
                );
                return group.length > 0 ? (
                  <section
                    key={String(pinned)}
                    aria-label={pinned ? "Pinned projects" : "Other projects"}
                  >
                    <h3 className={styles.groupTitle}>
                      {pinned ? "Pinned" : "Other projects"}
                    </h3>
                    {group.map((worker) => (
                      <ProjectCard
                        daemonBase={daemonBase}
                        key={worker.project_id}
                        onMutated={() => void refetch()}
                        worker={worker}
                      />
                    ))}
                  </section>
                ) : null;
              })}
            </div>
          ) : null}
          {missing.length > 0 ? (
            <MissingProjectsSection
              onMutated={() => void refetch()}
              workers={missing}
            />
          ) : null}
        </>
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
