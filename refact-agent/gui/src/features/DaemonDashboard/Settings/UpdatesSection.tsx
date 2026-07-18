import { useEffect, useState } from "react";
import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import { Download, RefreshCw, RotateCw } from "lucide-react";

import {
  Badge,
  Button,
  FieldError,
  SettingItem,
  Surface,
} from "../../../components/ui";
import {
  useGetDaemonInfoQuery,
  useGetDaemonUpdateStatusQuery,
  useInstallDaemonUpdateMutation,
  useLazyCheckDaemonUpdateQuery,
  useRestartDaemonMutation,
} from "../../../services/refact/daemon";
import { SettingsGroup } from "../../Settings/SettingsSection";
import styles from "./SettingsPage.module.css";

const UPDATE_POLLING_INTERVAL_MS = 500;
const RECONNECT_TIMEOUT_MS = 30_000;

function errorMessage(error: unknown, fallback: string): string {
  const queryError = error as FetchBaseQueryError | undefined;
  if (queryError && "data" in queryError) {
    const data = queryError.data;
    if (typeof data === "object" && data !== null && "error" in data) {
      const message = (data as { error?: unknown }).error;
      if (typeof message === "string" && message) return message;
    }
  }
  return fallback;
}

export function UpdatesSection() {
  const [checkUpdates, checkState] = useLazyCheckDaemonUpdateQuery();
  const [installUpdate, installState] = useInstallDaemonUpdateMutation();
  const [restartDaemon, restartState] = useRestartDaemonMutation();
  const [pollUpdates, setPollUpdates] = useState(false);
  const [reconnecting, setReconnecting] = useState(false);
  const [restartBaseline, setRestartBaseline] = useState<number | null>(null);
  const [reconnectError, setReconnectError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const statusQuery = useGetDaemonUpdateStatusQuery(undefined, {
    pollingInterval: pollUpdates ? UPDATE_POLLING_INTERVAL_MS : 0,
  });
  const infoQuery = useGetDaemonInfoQuery(undefined, {
    pollingInterval: reconnecting ? UPDATE_POLLING_INTERVAL_MS : 0,
  });

  const updateStatus = statusQuery.data;
  const activePhase =
    updateStatus?.phase === "checking" || updateStatus?.phase === "downloading";
  const installed = updateStatus?.phase === "restarting";

  useEffect(() => {
    if (activePhase) setPollUpdates(true);
    if (updateStatus?.phase === "failed" || installed) setPollUpdates(false);
  }, [activePhase, installed, updateStatus?.phase]);

  useEffect(() => {
    if (!reconnecting) return;
    const nextStartedAt = infoQuery.data?.status.started_at_ms;
    if (
      infoQuery.isSuccess &&
      typeof nextStartedAt === "number" &&
      (restartBaseline === null || nextStartedAt !== restartBaseline)
    ) {
      setReconnecting(false);
      setReconnectError(null);
    }
  }, [
    infoQuery.data?.status.started_at_ms,
    infoQuery.isSuccess,
    reconnecting,
    restartBaseline,
  ]);

  useEffect(() => {
    if (!reconnecting) return;
    const timeout = window.setTimeout(() => {
      setReconnecting(false);
      setReconnectError(
        "The daemon did not reconnect. Check the daemon and try again.",
      );
    }, RECONNECT_TIMEOUT_MS);
    return () => window.clearTimeout(timeout);
  }, [reconnecting]);

  async function check() {
    setActionError(null);
    try {
      await checkUpdates({ refresh: true }).unwrap();
    } catch (error) {
      setActionError(errorMessage(error, "Could not check for updates."));
    }
  }

  async function install() {
    setActionError(null);
    try {
      await installUpdate(
        checkState.data?.latest_version
          ? { version: checkState.data.latest_version }
          : {},
      ).unwrap();
      setPollUpdates(true);
      void statusQuery.refetch();
    } catch (error) {
      setActionError(errorMessage(error, "Could not install the update."));
    }
  }

  async function restart() {
    setActionError(null);
    setReconnectError(null);
    setRestartBaseline(infoQuery.data?.status.started_at_ms ?? null);
    setReconnecting(true);
    try {
      await restartDaemon(undefined).unwrap();
    } catch {
      void infoQuery.refetch();
    }
  }

  const phaseTone =
    updateStatus?.phase === "failed"
      ? "danger"
      : activePhase || reconnecting
        ? "accent"
        : installed
          ? "success"
          : "muted";

  return (
    <SettingsGroup title="Updates">
      <Surface className={styles.sectionSurface} variant="glass">
        <SettingItem
          title="Release channel"
          description="Check the configured daemon release source for a newer version."
          control={
            <Button
              leftIcon={RefreshCw}
              loading={checkState.isFetching}
              onClick={() => void check()}
              size="sm"
              variant="soft"
            >
              Check for updates
            </Button>
          }
        />
        {checkState.data ? (
          <SettingItem
            title={
              checkState.data.update_available
                ? `Version ${checkState.data.latest_version ?? "available"}`
                : "Daemon is up to date"
            }
            description={`Current version: ${checkState.data.current_version}`}
            control={
              checkState.data.update_available ? (
                <Button
                  leftIcon={Download}
                  loading={installState.isLoading}
                  onClick={() => void install()}
                  size="sm"
                  variant="primary"
                >
                  Install update
                </Button>
              ) : (
                <Badge tone="success" variant="soft">
                  Current
                </Badge>
              )
            }
          />
        ) : null}
        {updateStatus && updateStatus.phase !== "idle" ? (
          <div aria-live="polite" className={styles.updateProgress}>
            <Badge tone={phaseTone} variant="soft">
              {installed ? "installed" : updateStatus.phase}
            </Badge>
            <span>
              {updateStatus.detail ??
                (activePhase
                  ? "Update in progress…"
                  : "Update status changed.")}
            </span>
          </div>
        ) : null}
        {installed ? (
          <SettingItem
            title="Restart to finish"
            description="Restart the daemon, then wait for this dashboard to reconnect."
            control={
              <Button
                leftIcon={RotateCw}
                loading={restartState.isLoading || reconnecting}
                onClick={() => void restart()}
                size="sm"
                variant="primary"
              >
                Restart daemon
              </Button>
            }
          />
        ) : null}
        {reconnecting ? (
          <p aria-live="polite" className={styles.notice}>
            Reconnecting to the daemon…
          </p>
        ) : null}
        {!reconnecting && restartBaseline !== null && !reconnectError ? (
          <p aria-live="polite" className={styles.successNotice}>
            Daemon reconnected.
          </p>
        ) : null}
        {actionError ? <FieldError>{actionError}</FieldError> : null}
        {reconnectError ? <FieldError>{reconnectError}</FieldError> : null}
      </Surface>
    </SettingsGroup>
  );
}
