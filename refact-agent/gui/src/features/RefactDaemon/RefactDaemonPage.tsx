import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import {
  ArrowLeft,
  Lock,
  Power,
  RefreshCw,
  RotateCw,
  Server,
} from "lucide-react";
import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";

import {
  Badge,
  Button,
  ButtonGroup,
  DataTable,
  EmptyState,
  ErrorState,
  FieldError,
  FieldRow,
  FieldStack,
  FieldSwitch,
  FieldText,
  Icon,
  LoadingState,
  StatusDot,
  Surface,
} from "../../components/ui";
import type { DataTableColumn, StatusDotStatus } from "../../components/ui";
import { useConfig } from "../../hooks";
import {
  resolveDaemonBaseUrl,
  type DaemonRelease,
  type DaemonSettings,
  type DaemonSettingsUpdate,
  type DaemonUrls,
  type DaemonWorker,
  useGetDaemonSettingsQuery,
  useGetDaemonInfoQuery,
  useGetDaemonUpdateStatusQuery,
  useInstallDaemonUpdateMutation,
  useLazyCheckDaemonUpdateQuery,
  useRestartDaemonMutation,
  useShutdownDaemonMutation,
  useUpdateDaemonSettingsMutation,
} from "../../services/refact/daemon";
import styles from "./RefactDaemonPage.module.css";

const MINUTE_SECONDS = 60;
const HOUR_SECONDS = MINUTE_SECONDS * 60;
const DAY_SECONDS = HOUR_SECONDS * 24;
const SHORT_SHA_LENGTH = 12;
const DAEMON_POLLING_INTERVAL_MS = 3000;
const UPDATE_POLLING_INTERVAL_MS = 1000;

export type RefactDaemonPageProps = {
  backFromDaemon: () => void;
};

function isFiniteNumber(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function compareVersionsDesc(left: string, right: string): number {
  const leftParts = left.split(".").map((part) => Number.parseInt(part, 10));
  const rightParts = right.split(".").map((part) => Number.parseInt(part, 10));
  const length = Math.max(leftParts.length, rightParts.length);
  for (let index = 0; index < length; index += 1) {
    const leftValue = Number.isFinite(leftParts[index]) ? leftParts[index] : 0;
    const rightValue = Number.isFinite(rightParts[index])
      ? rightParts[index]
      : 0;
    if (leftValue !== rightValue) return rightValue - leftValue;
  }
  return right.localeCompare(left);
}

function daemonUrlEntries(urls: DaemonUrls): [string, string][] {
  const entries: [string, string][] = [
    ["Loopback", urls.loopback],
    ["mDNS", urls.mdns],
  ];
  return entries.filter(([, url]) => url.trim().length > 0);
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

function formatDate(value: string | null): string {
  if (!value) return "Unknown";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Unknown";
  return date.toLocaleDateString();
}

function getErrorMessage(error: unknown, fallback: string): string {
  const maybeError = error as FetchBaseQueryError | undefined;
  if (maybeError && "data" in maybeError) {
    const data = maybeError.data;
    if (typeof data === "object" && data !== null && "error" in data) {
      const message = (data as { error?: unknown }).error;
      if (typeof message === "string" && message.length > 0) return message;
    }
  }
  return fallback;
}

function settingsToForm(settings: DaemonSettings): DaemonSettingsForm {
  return {
    lan_enabled: settings.lan_enabled,
    mdns_enabled: settings.mdns_enabled,
    auth_enabled: settings.auth_enabled,
    username: settings.username ?? "",
    password: "",
  };
}

type DaemonSettingsForm = {
  lan_enabled: boolean;
  mdns_enabled: boolean;
  auth_enabled: boolean;
  username: string;
  password: string;
};

function buildSettingsPayload(form: DaemonSettingsForm): DaemonSettingsUpdate {
  const payload: DaemonSettingsUpdate = {
    lan_enabled: form.lan_enabled,
    mdns_enabled: form.mdns_enabled,
    auth_enabled: form.auth_enabled,
  };
  const username = form.username.trim();
  if (username.length > 0) payload.username = username;
  if (form.password.length > 0) payload.password = form.password;
  return payload;
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

function NetworkAccessCard() {
  const { data, isLoading } = useGetDaemonSettingsQuery(undefined);
  const [updateSettings, updateState] = useUpdateDaemonSettingsMutation();
  const [form, setForm] = useState<DaemonSettingsForm | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const settings = data?.settings ?? null;
  const authHidden = data?.access === "auth_hidden";

  useEffect(() => {
    if (settings) setForm(settingsToForm(settings));
  }, [settings]);

  const unchanged = useMemo(() => {
    if (!settings || !form) return true;
    const original = settingsToForm(settings);
    return (
      original.lan_enabled === form.lan_enabled &&
      original.mdns_enabled === form.mdns_enabled &&
      original.auth_enabled === form.auth_enabled &&
      original.username === form.username &&
      form.password.length === 0
    );
  }, [form, settings]);

  const lanAuthHint =
    form?.lan_enabled === true &&
    (!form.auth_enabled || form.username.trim().length === 0 || form.password.length === 0) &&
    !(settings?.has_password === true && form.password.length === 0);

  async function saveSettings() {
    if (!form) return;
    setNotice(null);
    try {
      await updateSettings(buildSettingsPayload(form)).unwrap();
      setNotice("Daemon is restarting…");
    } catch {
      setNotice(null);
    }
  }

  return (
    <Surface className={styles.controlCard} variant="glass">
      <div className={styles.cardHeader}>
        <div>
          <h2>Network &amp; Access</h2>
          <p>Configure LAN binding, discovery, and daemon authentication.</p>
        </div>
        <Badge tone={authHidden ? "warning" : "muted"} variant="soft">
          {authHidden ? "Hidden by auth" : "Settings"}
        </Badge>
      </div>
      {isLoading || !data ? (
        <LoadingState kind="skeleton" label="Loading daemon settings" />
      ) : authHidden ? (
        <EmptyState
          icon={Lock}
          title="Settings hidden — daemon auth enabled"
          description="Authenticate with the daemon before changing network access settings."
        />
      ) : form && settings ? (
        <div className={styles.formStack}>
          <FieldRow
            label="Listen on 0.0.0.0"
            helper="Allow LAN clients to reach the daemon."
            control={
              <FieldSwitch
                aria-label="Listen on 0.0.0.0"
                checked={form.lan_enabled}
                onChange={(lan_enabled) => setForm({ ...form, lan_enabled })}
              />
            }
          />
          <FieldRow
            label="mDNS discovery"
            helper="Advertise this daemon as a local service."
            control={
              <FieldSwitch
                aria-label="mDNS discovery"
                checked={form.mdns_enabled}
                onChange={(mdns_enabled) => setForm({ ...form, mdns_enabled })}
              />
            }
          />
          <FieldRow
            label="Authentication"
            helper="Require username and password for protected daemon APIs."
            control={
              <FieldSwitch
                aria-label="Authentication"
                checked={form.auth_enabled}
                onChange={(auth_enabled) => setForm({ ...form, auth_enabled })}
              />
            }
          />
          <div className={styles.formGrid}>
            <FieldStack label="Username" htmlFor="daemon-username">
              <FieldText
                id="daemon-username"
                value={form.username}
                onChange={(username) => setForm({ ...form, username })}
              />
            </FieldStack>
            <FieldStack
              label="Password"
              htmlFor="daemon-password"
              helper={
                settings.has_password && form.password.length === 0
                  ? "Leave empty to keep the current password."
                  : undefined
              }
            >
              <FieldText
                id="daemon-password"
                type="password"
                placeholder={
                  settings.has_password && form.password.length === 0
                    ? "unchanged"
                    : undefined
                }
                value={form.password}
                onChange={(password) => setForm({ ...form, password })}
              />
            </FieldStack>
          </div>
          {lanAuthHint ? (
            <p className={styles.helperText}>
              LAN access usually requires authentication with username and
              password; the backend will reject invalid combinations.
            </p>
          ) : null}
          {updateState.error ? (
            <FieldError>
              {getErrorMessage(
                updateState.error,
                "Could not save daemon settings.",
              )}
            </FieldError>
          ) : null}
          {notice ? <p className={styles.notice}>{notice}</p> : null}
          <div className={styles.urlList}>
            <span className={styles.sectionLabel}>Reachable URLs</span>
            {daemonUrlEntries(settings.urls).length > 0 ? (
              <ul>
                {daemonUrlEntries(settings.urls).map(([label, url]) => (
                  <li key={label}>
                    <span className={styles.mutedValue}>{label}</span> {url}
                  </li>
                ))}
              </ul>
            ) : (
              <span className={styles.mutedValue}>No URLs reported.</span>
            )}
          </div>
          <Button
            loading={updateState.isLoading}
            disabled={unchanged || updateState.isLoading}
            onClick={() => void saveSettings()}
            size="sm"
            variant="primary"
          >
            Save settings
          </Button>
        </div>
      ) : null}
    </Surface>
  );
}

function ActionsCard() {
  const [restart, restartState] = useRestartDaemonMutation();
  const [shutdown, shutdownState] = useShutdownDaemonMutation();
  const [confirm, setConfirm] = useState<"restart" | "shutdown" | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  async function runRestart() {
    await restart(undefined).unwrap();
    setNotice("Daemon is restarting…");
    setConfirm(null);
  }

  async function runShutdown() {
    await shutdown({ reason: "gui_shutdown" }).unwrap();
    setNotice("Daemon is stopping. This page will show unreachable shortly.");
    setConfirm(null);
  }

  return (
    <Surface className={styles.controlCard} variant="glass">
      <div className={styles.cardHeader}>
        <div>
          <h2>Actions</h2>
          <p>Restart or stop the daemon process from the GUI.</p>
        </div>
        <Badge tone="warning" variant="soft">
          Control
        </Badge>
      </div>
      <div className={styles.actionStack}>
        <Button
          leftIcon={RotateCw}
          loading={restartState.isLoading}
          onClick={() => setConfirm("restart")}
          size="sm"
          variant="soft"
        >
          Restart daemon
        </Button>
        <Button
          leftIcon={Power}
          loading={shutdownState.isLoading}
          onClick={() => setConfirm("shutdown")}
          size="sm"
          variant="danger"
        >
          Shutdown daemon
        </Button>
      </div>
      {confirm ? (
        <div className={styles.confirmBox}>
          <p>
            {confirm === "restart"
              ? "Restart the daemon now?"
              : "Shutdown the daemon now?"}
          </p>
          <ButtonGroup>
            <Button size="sm" variant="soft" onClick={() => setConfirm(null)}>
              Cancel
            </Button>
            <Button
              size="sm"
              variant={confirm === "restart" ? "primary" : "danger"}
              loading={restartState.isLoading || shutdownState.isLoading}
              onClick={() =>
                void (confirm === "restart" ? runRestart() : runShutdown())
              }
            >
              Confirm {confirm}
            </Button>
          </ButtonGroup>
        </div>
      ) : null}
      {restartState.error ? (
        <FieldError>
          {getErrorMessage(restartState.error, "Could not restart daemon.")}
        </FieldError>
      ) : null}
      {shutdownState.error ? (
        <FieldError>
          {getErrorMessage(shutdownState.error, "Could not shutdown daemon.")}
        </FieldError>
      ) : null}
      {notice ? <p className={styles.notice}>{notice}</p> : null}
    </Surface>
  );
}

function UpdatesCard({
  version,
  executableSha256,
}: {
  version: string;
  executableSha256?: string;
}) {
  const [checkUpdates, checkState] = useLazyCheckDaemonUpdateQuery();
  const [installUpdate, installState] = useInstallDaemonUpdateMutation();
  const [shouldPollUpdates, setShouldPollUpdates] = useState(false);
  const statusQuery = useGetDaemonUpdateStatusQuery(undefined, {
    pollingInterval: shouldPollUpdates ? UPDATE_POLLING_INTERVAL_MS : 0,
  });
  const [installError, setInstallError] = useState<string | null>(null);

  const updateStatus = statusQuery.data;
  const statusPolling = updateStatus
    ? ["checking", "downloading", "restarting"].includes(updateStatus.phase)
    : false;
  const releases = useMemo(
    () =>
      [...(checkState.data?.releases ?? [])].sort((left, right) =>
        compareVersionsDesc(left.version, right.version),
      ),
    [checkState.data?.releases],
  );

  useEffect(() => {
    if (!updateStatus) return;
    setShouldPollUpdates(statusPolling);
  }, [statusPolling, updateStatus]);

  async function install(versionTarget?: string) {
    setInstallError(null);
    try {
      await installUpdate(versionTarget ? { version: versionTarget } : {}).unwrap();
      setShouldPollUpdates(true);
      void statusQuery.refetch();
    } catch (error) {
      setInstallError(getErrorMessage(error, "update already in progress"));
    }
  }

  return (
    <Surface className={styles.controlCard} variant="glass">
      <div className={styles.cardHeader}>
        <div>
          <h2>Updates</h2>
          <p>Check for daemon releases and install an available update.</p>
        </div>
        <Badge
          tone={checkState.data?.update_available ? "success" : "muted"}
          variant="soft"
        >
          {checkState.data?.update_available ? "Update available" : "Current"}
        </Badge>
      </div>
      <dl className={styles.statsGrid}>
        <StatItem label="Current version" value={version || "Unknown"} />
        <StatItem
          label="Executable SHA"
          value={
            executableSha256
              ? executableSha256.slice(0, SHORT_SHA_LENGTH)
              : "—"
          }
        />
      </dl>
      <Button
        loading={checkState.isFetching}
        onClick={() => void checkUpdates({ refresh: true })}
        size="sm"
        variant="soft"
      >
        Check for updates
      </Button>
      {checkState.error ? (
        <FieldError>
          {getErrorMessage(checkState.error, "Could not check for updates.")}
        </FieldError>
      ) : null}
      {checkState.data ? (
        <div className={styles.updateSummary}>
          <p>Latest version: {checkState.data.latest_version ?? "None reported"}</p>
          {checkState.data.update_available ? (
            <Button
              size="sm"
              variant="primary"
              loading={installState.isLoading}
              onClick={() =>
                void install(checkState.data?.latest_version ?? undefined)
              }
            >
              Install latest
            </Button>
          ) : null}
        </div>
      ) : null}
      {releases.length > 0 ? (
        <ul className={styles.releaseList}>
          {releases.map((release: DaemonRelease) => (
            <li key={release.version}>
              <div>
                <span>{release.version}</span>
                <span className={styles.mutedValue}>{formatDate(release.published_at)}</span>
                {release.prerelease ? (
                  <Badge tone="warning" size="xs" variant="soft">
                    Prerelease
                  </Badge>
                ) : null}
              </div>
              <Button
                size="sm"
                variant="soft"
                loading={installState.isLoading}
                onClick={() => void install(release.version)}
              >
                Install
              </Button>
            </li>
          ))}
        </ul>
      ) : null}
      {updateStatus ? (
        <div className={styles.updateStatus}>
          <span className={styles.sectionLabel}>Update status</span>
          <Badge
            tone={
              updateStatus.phase === "failed"
                ? "danger"
                : statusPolling
                  ? "accent"
                  : "muted"
            }
            variant="soft"
          >
            {updateStatus.phase}
          </Badge>
          {updateStatus.target_version ? (
            <span>Target: {updateStatus.target_version}</span>
          ) : null}
          {updateStatus.detail ? <span>{updateStatus.detail}</span> : null}
          {updateStatus.phase === "restarting" ? (
            <p className={styles.notice}>Daemon is restarting…</p>
          ) : null}
        </div>
      ) : null}
      {installError ? <FieldError>{installError}</FieldError> : null}
    </Surface>
  );
}

export function RefactDaemonPage({ backFromDaemon }: RefactDaemonPageProps) {
  const config = useConfig();
  const daemonBaseUrl = resolveDaemonBaseUrl(config);
  const { data, error, isLoading, isFetching, refetch } = useGetDaemonInfoQuery(
    undefined,
    { pollingInterval: DAEMON_POLLING_INTERVAL_MS },
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

          <NetworkAccessCard />
          <ActionsCard />
          <UpdatesCard
            version={status.version}
            executableSha256={status.executable_sha256}
          />
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
