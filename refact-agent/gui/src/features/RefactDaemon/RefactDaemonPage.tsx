import { useMemo } from "react";
import type { ReactNode } from "react";
import { ArrowLeft, Lock, RefreshCw, Server } from "lucide-react";

import {
  Badge,
  Button,
  ButtonGroup,
  DataTable,
  EmptyState,
  ErrorState,
  Icon,
  LoadingState,
  StatusDot,
  Surface,
} from "../../components/ui";
import type { DataTableColumn, StatusDotStatus } from "../../components/ui";
import { useConfig } from "../../hooks";
import {
  resolveDaemonBaseUrl,
  type DaemonWorker,
  useGetDaemonInfoQuery,
} from "../../services/refact/daemon";
import styles from "./RefactDaemonPage.module.css";

const MINUTE_SECONDS = 60;
const HOUR_SECONDS = MINUTE_SECONDS * 60;
const DAY_SECONDS = HOUR_SECONDS * 24;

export type RefactDaemonPageProps = {
  backFromDaemon: () => void;
};

function isFiniteNumber(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function formatNullableNumber(value: number | null | undefined): string {
  return isFiniteNumber(value) ? String(value) : "—";
}

function cronPendingEntries(
  cronPending: Record<string, number>,
): [string, number][] {
  return Object.entries(cronPending).sort(([left], [right]) =>
    left.localeCompare(right),
  );
}

function formatCronPendingCount(cronPending: Record<string, number>): string {
  const count = cronPendingEntries(cronPending).length;
  return `${count} pending`;
}

function formatUptime(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds <= 0) return "0s";

  const days = Math.floor(totalSeconds / DAY_SECONDS);
  const hours = Math.floor((totalSeconds % DAY_SECONDS) / HOUR_SECONDS);
  const minutes = Math.floor((totalSeconds % HOUR_SECONDS) / MINUTE_SECONDS);
  const seconds = Math.floor(totalSeconds % MINUTE_SECONDS);

  if (days > 0) return `${days}d ${hours}h ${minutes}m`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

function formatTimestamp(timestampMs: number | null | undefined): string {
  if (!isFiniteNumber(timestampMs) || timestampMs <= 0) return "Unknown";
  return new Date(timestampMs).toLocaleString();
}

function workerStatus(state: string): StatusDotStatus {
  const normalized = state.toLowerCase();
  if (
    normalized === "running" ||
    normalized === "active" ||
    normalized === "ready" ||
    normalized === "busy"
  ) {
    return "running";
  }
  if (normalized === "idle") return "idle";
  if (normalized === "paused") return "paused";
  if (
    normalized === "error" ||
    normalized === "failed" ||
    normalized === "crashed" ||
    normalized === "stopped"
  ) {
    return "error";
  }
  return "warning";
}

function WorkerState({ state }: { state: string }) {
  const status = workerStatus(state);
  const tone =
    status === "running"
      ? "success"
      : status === "error"
        ? "danger"
        : status === "warning"
          ? "warning"
          : "muted";

  return (
    <span className={styles.stateCell}>
      <StatusDot
        aria-label={`${state} worker state`}
        status={status}
        pulse={status === "running"}
      />
      <Badge tone={tone} size="xs" variant="soft">
        {state || "unknown"}
      </Badge>
    </span>
  );
}

function PathValue({ value }: { value: string }) {
  return (
    <span className={styles.pathValue} title={value}>
      {value || "—"}
    </span>
  );
}

function ErrorValue({ value }: { value: string | null }) {
  if (!value) return <span className={styles.mutedValue}>—</span>;
  return (
    <span className={styles.errorValue} title={value}>
      {value}
    </span>
  );
}

function StatItem({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className={styles.statItem}>
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

function CronPendingValue({
  cronPending,
}: {
  cronPending: Record<string, number>;
}) {
  const entries = cronPendingEntries(cronPending);
  if (entries.length === 0) return "0 pending";

  return (
    <span className={styles.cronPendingValue}>
      <span>{formatCronPendingCount(cronPending)}</span>
      <span className={styles.cronPendingList}>
        {entries.map(([slug, pendingMs]) => (
          <span className={styles.cronPendingItem} key={slug}>
            <span>{slug}</span>
            <span>{pendingMs} ms</span>
          </span>
        ))}
      </span>
    </span>
  );
}

export function RefactDaemonPage({ backFromDaemon }: RefactDaemonPageProps) {
  const config = useConfig();
  const daemonBaseUrl = resolveDaemonBaseUrl(config);
  const { data, error, isLoading, isFetching, refetch } = useGetDaemonInfoQuery(
    undefined,
    { pollingInterval: 3000 },
  );

  const columns = useMemo<DataTableColumn<DaemonWorker>[]>(
    () => [
      {
        id: "slug",
        header: "Slug",
        cell: (worker) => <PathValue value={worker.slug} />,
        sortValue: (worker) => worker.slug,
      },
      {
        id: "root",
        header: "Root",
        cell: (worker) => <PathValue value={worker.root} />,
        sortValue: (worker) => worker.root,
      },
      {
        id: "state",
        header: "State",
        cell: (worker) => <WorkerState state={worker.state} />,
        sortValue: (worker) => worker.state,
      },
      {
        id: "pid",
        header: "PID",
        cell: (worker) => formatNullableNumber(worker.pid),
        sortValue: (worker) => worker.pid,
        align: "end",
      },
      {
        id: "http_port",
        header: "HTTP",
        cell: (worker) => formatNullableNumber(worker.http_port),
        sortValue: (worker) => worker.http_port,
        align: "end",
      },
      {
        id: "lsp_port",
        header: "LSP",
        cell: (worker) => formatNullableNumber(worker.lsp_port),
        sortValue: (worker) => worker.lsp_port,
        align: "end",
      },
      {
        id: "lsp_clients",
        header: "Clients",
        cell: (worker) => formatNullableNumber(worker.lsp_clients),
        sortValue: (worker) => worker.lsp_clients,
        align: "end",
      },
      {
        id: "busy_chats",
        header: "Busy chats",
        cell: (worker) => formatNullableNumber(worker.busy_chats),
        sortValue: (worker) => worker.busy_chats,
        align: "end",
      },
      {
        id: "exec_running",
        header: "Exec",
        cell: (worker) => formatNullableNumber(worker.exec_running),
        sortValue: (worker) => worker.exec_running,
        align: "end",
      },
      {
        id: "live_proxy_streams",
        header: "Streams",
        cell: (worker) => formatNullableNumber(worker.live_proxy_streams),
        sortValue: (worker) => worker.live_proxy_streams,
        align: "end",
      },
      {
        id: "last_error",
        header: "Last error",
        cell: (worker) => <ErrorValue value={worker.last_error} />,
        sortValue: (worker) => worker.last_error ?? "",
      },
    ],
    [],
  );

  const status = data?.status;
  const workers = data?.workers ?? [];
  const workersHiddenByAuth = data?.workersAccess === "auth_hidden";
  const workersEmptyMessage = workersHiddenByAuth ? (
    <EmptyState
      icon={Lock}
      title="Workers hidden — daemon auth enabled"
      description="The daemon status endpoint is public, but this daemon requires its auth token before it exposes worker details."
    />
  ) : (
    "No workers reported by daemon."
  );

  return (
    <main className={styles.page}>
      <header className={styles.header}>
        <ButtonGroup className={styles.headerActions}>
          <Button leftIcon={ArrowLeft} onClick={backFromDaemon} size="sm">
            Back
          </Button>
        </ButtonGroup>
        <div className={styles.headingBlock}>
          <div className={styles.eyebrow}>
            <Icon icon={Server} size="sm" />
            Runtime status
          </div>
          <h1>Refact Daemon</h1>
          <p>
            Live status from the daemon root used by this GUI. Auto-refreshes
            every few seconds.
          </p>
        </div>
        <ButtonGroup className={styles.headerActions}>
          <Button
            leftIcon={RefreshCw}
            loading={isFetching}
            onClick={() => void refetch()}
            size="sm"
            variant="soft"
          >
            Refresh
          </Button>
        </ButtonGroup>
      </header>

      {isLoading ? (
        <LoadingState kind="skeleton" label="Loading daemon status" />
      ) : error && !data ? (
        <ErrorState
          title="Daemon unreachable"
          description="The GUI could not reach the daemon status endpoint. Check that the daemon is running and reachable from this webview."
          retry={
            <Button
              leftIcon={RefreshCw}
              loading={isFetching}
              onClick={() => void refetch()}
              size="sm"
            >
              Retry
            </Button>
          }
        />
      ) : status ? (
        <div className={styles.content}>
          <Surface className={styles.statusCard} variant="glass">
            <div className={styles.cardHeader}>
              <div>
                <h2>Daemon status</h2>
                <p>{daemonBaseUrl}</p>
              </div>
              <Badge tone={isFetching ? "accent" : "success"} variant="soft">
                {isFetching ? "Refreshing" : "Live"}
              </Badge>
            </div>
            <dl className={styles.statsGrid}>
              <StatItem label="Version" value={status.version || "Unknown"} />
              <StatItem label="PID" value={formatNullableNumber(status.pid)} />
              <StatItem
                label="Port"
                value={formatNullableNumber(status.port)}
              />
              <StatItem
                label="Uptime"
                value={formatUptime(status.uptime_secs)}
              />
              <StatItem
                label="Started"
                value={formatTimestamp(status.started_at_ms)}
              />
              <StatItem
                label="Workers"
                value={formatNullableNumber(status.workers)}
              />
              <StatItem
                label="Cron pending"
                value={<CronPendingValue cronPending={status.cron_pending} />}
              />
              <StatItem label="Base URL" value={daemonBaseUrl} />
            </dl>
          </Surface>

          <Surface className={styles.workersCard} variant="glass">
            <div className={styles.cardHeader}>
              <div>
                <h2>Workers</h2>
                <p>Per-project daemon workers currently known to the daemon.</p>
              </div>
              <Badge
                tone={workersHiddenByAuth ? "warning" : "muted"}
                variant="soft"
              >
                {workersHiddenByAuth
                  ? "Hidden by auth"
                  : `${workers.length} shown`}
              </Badge>
            </div>
            <DataTable
              columns={columns}
              rows={workers}
              getRowId={(worker, index) =>
                `${worker.project_id || worker.slug || "worker"}-${index}`
              }
              caption="Daemon workers"
              emptyMessage={workersEmptyMessage}
              enableSorting
              wide
            />
          </Surface>
        </div>
      ) : (
        <EmptyState
          icon={Server}
          title="Daemon endpoint unknown"
          description="No daemon status has been reported yet. Refresh after the connection settings finish loading."
          action={
            <Button
              leftIcon={RefreshCw}
              loading={isFetching}
              onClick={() => void refetch()}
              size="sm"
            >
              Refresh
            </Button>
          }
        />
      )}
    </main>
  );
}
