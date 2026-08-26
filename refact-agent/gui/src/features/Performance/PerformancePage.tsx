import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowLeft, Gauge, RefreshCw, RotateCcw } from "lucide-react";

import {
  Badge,
  Button,
  ButtonGroup,
  EmptyState,
  ErrorState,
  LoadingState,
  Surface,
} from "../../components/ui";
import {
  useGetPerformanceTelemetryQuery,
  useResetPerformanceTelemetryMutation,
  useSetPerformanceTelemetryEnabledMutation,
} from "../../services/refact/performance";
import {
  formatComponentName,
  formatTimestamp,
  formatUptime,
} from "./performanceFormatters";
import { TelemetryPanel } from "./TelemetryPanel";
import { TrajectorySettingsPanel } from "./TrajectorySettingsPanel";
import styles from "./PerformancePage.module.css";

export const PERFORMANCE_POLLING_INTERVAL_MS = 5_000;

function isDocumentVisible(): boolean {
  return (
    typeof document === "undefined" || document.visibilityState !== "hidden"
  );
}

function useDocumentVisibility(): boolean {
  const [visible, setVisible] = useState(isDocumentVisible);

  useEffect(() => {
    const onVisibilityChange = () => setVisible(isDocumentVisible());
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () =>
      document.removeEventListener("visibilitychange", onVisibilityChange);
  }, []);

  return visible;
}

export type PerformancePageProps = {
  onBack: () => void;
};

export function PerformancePage({ onBack }: PerformancePageProps) {
  const visible = useDocumentVisibility();
  const visibilityInitialized = useRef(false);
  const {
    data: telemetry,
    isError,
    isFetching,
    isLoading,
    refetch,
  } = useGetPerformanceTelemetryQuery(undefined, {
    pollingInterval: visible ? PERFORMANCE_POLLING_INTERVAL_MS : 0,
    refetchOnFocus: true,
    refetchOnReconnect: true,
    skipPollingIfUnfocused: true,
  });
  const [setEnabled, setEnabledState] =
    useSetPerformanceTelemetryEnabledMutation();
  const [resetTelemetry, resetState] = useResetPerformanceTelemetryMutation();
  const [actionError, setActionError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    setActionError(null);
    void refetch();
  }, [refetch]);

  useEffect(() => {
    if (!visibilityInitialized.current) {
      visibilityInitialized.current = true;
      return;
    }
    if (visible) refresh();
  }, [refresh, visible]);

  const changeCollection = useCallback(
    async (enabled: boolean) => {
      setActionError(null);
      try {
        await setEnabled(enabled).unwrap();
        await refetch();
      } catch {
        setActionError("Could not update telemetry collection.");
      }
    },
    [refetch, setEnabled],
  );

  const reset = useCallback(async () => {
    setActionError(null);
    try {
      await resetTelemetry(undefined).unwrap();
      await refetch();
    } catch {
      setActionError("Could not reset telemetry.");
    }
  }, [refetch, resetTelemetry]);

  const collectionEnabled = telemetry?.enabled === true;
  const rollouts = Object.entries(telemetry?.rollout_switches ?? {});

  return (
    <main className={styles.page}>
      <header className={styles.header}>
        <Button leftIcon={ArrowLeft} onClick={onBack} size="sm" variant="ghost">
          Back
        </Button>
        <div className={styles.headingBlock}>
          <p className={styles.eyebrow}>Runtime observability</p>
          <h1>Performance</h1>
          <p>Aggregate-only engine telemetry for the current Refact runtime.</p>
        </div>
        <ButtonGroup className={styles.headerActions}>
          <Button
            leftIcon={RefreshCw}
            loading={isFetching}
            onClick={refresh}
            size="sm"
            variant="soft"
          >
            Refresh
          </Button>
          <Button
            leftIcon={RotateCcw}
            loading={resetState.isLoading}
            onClick={() => void reset()}
            size="sm"
            variant="danger"
          >
            Reset
          </Button>
        </ButtonGroup>
      </header>

      {isLoading ? (
        <LoadingState label="Loading performance telemetry" variant="full" />
      ) : isError ? (
        <ErrorState
          description="The engine did not return performance telemetry. Check the connection and try again."
          retry={
            <Button onClick={refresh} size="sm" variant="soft">
              Retry
            </Button>
          }
          title="Performance telemetry unavailable"
          variant="full"
        />
      ) : telemetry ? (
        <div className={styles.content}>
          <Surface className={styles.collectionCard} variant="glass">
            <div className={styles.cardHeader}>
              <div>
                <h2>Collection state</h2>
                <p>
                  Telemetry stays aggregate-only and can be enabled or disabled
                  for this engine process.
                </p>
              </div>
              <Badge
                tone={collectionEnabled ? "success" : "muted"}
                variant="soft"
              >
                {collectionEnabled ? "Enabled" : "Disabled"}
              </Badge>
            </div>
            <dl className={styles.collectionStats}>
              <div>
                <dt>Enabled since</dt>
                <dd>
                  {collectionEnabled
                    ? formatTimestamp(telemetry.collection_started_at_ms)
                    : "Not collecting"}
                </dd>
              </div>
              <div>
                <dt>Collection uptime</dt>
                <dd>
                  {collectionEnabled ? formatUptime(telemetry.uptime_ms) : "—"}
                </dd>
              </div>
              <div>
                <dt>Schema version</dt>
                <dd>{telemetry.schema_version ?? "—"}</dd>
              </div>
            </dl>
            <div className={styles.collectionActions}>
              <Button
                loading={setEnabledState.isLoading}
                onClick={() => void changeCollection(!collectionEnabled)}
                size="sm"
                variant={collectionEnabled ? "soft" : "primary"}
              >
                {collectionEnabled ? "Disable collection" : "Enable collection"}
              </Button>
            </div>
            {rollouts.length > 0 ? (
              <div className={styles.rolloutSwitches}>
                <h3>Rollout switches</h3>
                <ul>
                  {rollouts.map(([name, enabled]) => (
                    <li key={name}>
                      <span>{formatComponentName(name)}</span>
                      <Badge
                        tone={enabled ? "success" : "muted"}
                        size="xs"
                        variant="soft"
                      >
                        {enabled ? "On" : "Off"}
                      </Badge>
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
          </Surface>

          {actionError ? (
            <ErrorState
              description={actionError}
              title="Telemetry action failed"
            />
          ) : null}

          <TrajectorySettingsPanel />

          {!collectionEnabled ? (
            <EmptyState
              icon={Gauge}
              title="Telemetry collection is disabled"
              description="Enable it to collect bounded runtime aggregates for this engine process."
              action={
                <Button
                  loading={setEnabledState.isLoading}
                  onClick={() => void changeCollection(true)}
                  size="sm"
                  variant="primary"
                >
                  Enable collection
                </Button>
              }
              variant="full"
            />
          ) : (
            <TelemetryPanel telemetry={telemetry} />
          )}
        </div>
      ) : (
        <EmptyState
          icon={Gauge}
          title="Performance telemetry unavailable"
          description="Refresh after the engine connection finishes loading."
          action={
            <Button onClick={refresh} size="sm" variant="soft">
              Refresh
            </Button>
          }
          variant="full"
        />
      )}
    </main>
  );
}
