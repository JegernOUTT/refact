import { useEffect, useMemo, useState } from "react";
import {
  Activity,
  ChartNoAxesCombined,
  CircleDollarSign,
  Database,
  Gauge,
  Power,
  TriangleAlert,
} from "lucide-react";

import {
  Button,
  EmptyState,
  Icon,
  LoadingState,
  SegmentedControl,
  Surface,
} from "../../../components/ui";
import { useAppSelector } from "../../../hooks";
import {
  projectApiUrl,
  resolveDaemonBaseUrl,
  useListProjectsQuery,
  useRestartProjectMutation,
  type DaemonWorker,
} from "../../../services/refact/daemon";
import { selectConfig } from "../../Config/configSlice";
import { StatCard } from "../../StatsDashboard/components/StatCard";
import type { StatsSummary } from "../../StatsDashboard/types";
import {
  daysAgoIsoDate,
  todayIsoDate,
} from "../../StatsDashboard/utils/dateRange";
import {
  formatCostPrecise,
  formatNumber,
  formatRatioPercent,
  formatTokenCount,
} from "../../StatsDashboard/utils/formatters";
import { isReadyWorker } from "../Projects/projectRagStatus";
import {
  aggregateUsage,
  type AggregatedUsage,
  type ProjectUsageInput,
} from "./aggregate";
import { UsageCharts } from "./UsageCharts";
import styles from "./Usage.module.css";

const REQUEST_TIMEOUT_MS = 5_000;
const MAX_CONCURRENT_REQUESTS = 3;
const MAX_PROVIDER_PROBES = 3;
const LOW_PLAN_REMAINING_RATIO = 0.1;

type RangePreset = "7d" | "30d" | "90d";

const RANGE_OPTIONS = [
  { value: "7d", label: "7 days" },
  { value: "30d", label: "30 days" },
  { value: "90d", label: "90 days" },
];

const RANGE_DAYS: Record<RangePreset, number> = {
  "7d": 7,
  "30d": 30,
  "90d": 90,
};

type UsageFetchState =
  | { state: "loading" }
  | { state: "ready"; inputs: ProjectUsageInput[]; unavailableSlugs: string[] };

type TokenPlanWarning = {
  provider: string;
  message: string;
};

function isStatsSummary(data: unknown): data is StatsSummary {
  if (!data || typeof data !== "object") return false;
  const record = data as Record<string, unknown>;
  return (
    typeof record.totals === "object" &&
    record.totals !== null &&
    Array.isArray(record.by_model) &&
    Array.isArray(record.by_provider) &&
    Array.isArray(record.by_day)
  );
}

async function fetchJsonWithTimeout(url: string): Promise<unknown> {
  const controller = new AbortController();
  const timeout = window.setTimeout(
    () => controller.abort(),
    REQUEST_TIMEOUT_MS,
  );
  try {
    const response = await fetch(url, {
      credentials: "same-origin",
      signal: controller.signal,
    });
    if (!response.ok) throw new Error("Request failed");
    return (await response.json()) as unknown;
  } finally {
    window.clearTimeout(timeout);
  }
}

async function fetchProjectSummaries(
  daemonBase: string,
  workers: DaemonWorker[],
  query: string,
): Promise<{ inputs: ProjectUsageInput[]; unavailableSlugs: string[] }> {
  const inputs: ProjectUsageInput[] = [];
  const unavailableSlugs: string[] = [];
  let nextIndex = 0;

  async function run() {
    while (nextIndex < workers.length) {
      const worker = workers[nextIndex];
      nextIndex += 1;
      try {
        const data = await fetchJsonWithTimeout(
          `${projectApiUrl(
            daemonBase,
            worker.project_id,
            "/stats/llm/summary",
          )}${query}`,
        );
        if (isStatsSummary(data)) {
          inputs.push({
            projectId: worker.project_id,
            slug: worker.slug,
            summary: data,
          });
        } else {
          unavailableSlugs.push(worker.slug);
        }
      } catch {
        unavailableSlugs.push(worker.slug);
      }
    }
  }

  await Promise.all(
    Array.from(
      { length: Math.min(MAX_CONCURRENT_REQUESTS, workers.length) },
      run,
    ),
  );
  inputs.sort((left, right) => left.slug.localeCompare(right.slug));
  unavailableSlugs.sort((left, right) => left.localeCompare(right));
  return { inputs, unavailableSlugs };
}

function tokenPlanWarning(
  provider: string,
  payload: unknown,
): TokenPlanWarning | null {
  if (!payload || typeof payload !== "object") return null;
  const data = (payload as { data?: unknown }).data;
  if (!data || typeof data !== "object") return null;
  const record = data as Record<string, unknown>;
  const limit = typeof record.limit === "number" ? record.limit : null;
  const remaining =
    typeof record.remaining === "number" ? record.remaining : null;
  if (remaining !== null && remaining <= 0) {
    return { provider, message: `${provider}: token plan exhausted` };
  }
  if (
    limit !== null &&
    remaining !== null &&
    remaining <= limit * LOW_PLAN_REMAINING_RATIO
  ) {
    return {
      provider,
      message: `${provider}: ${formatCostPrecise(
        remaining,
      )} of ${formatCostPrecise(limit)} plan remaining`,
    };
  }
  return null;
}

type NotCountedRowProps = {
  worker: DaemonWorker;
  reason: string;
  onWoke?: () => void;
};

function NotCountedRow({ worker, reason, onWoke }: NotCountedRowProps) {
  const [restart, restartState] = useRestartProjectMutation();
  return (
    <li className={styles.stoppedRow}>
      <span className={styles.stoppedSlug}>{worker.slug}</span>
      <span className={styles.stoppedNote}>{reason}</span>
      {onWoke && (
        <Button
          leftIcon={Power}
          loading={restartState.isLoading}
          onClick={() =>
            void restart(worker.project_id)
              .unwrap()
              .then(onWoke)
              .catch(() => undefined)
          }
          size="sm"
          variant="primary"
        >
          Wake
        </Button>
      )}
    </li>
  );
}

type UsageContentProps = {
  aggregated: AggregatedUsage;
  warnings: TokenPlanWarning[];
};

function UsageContent({ aggregated, warnings }: UsageContentProps) {
  const [costAsc, setCostAsc] = useState(false);
  const projectRows = useMemo(
    () =>
      [...aggregated.by_project].sort((left, right) => {
        const leftCost = left.total_cost_usd ?? 0;
        const rightCost = right.total_cost_usd ?? 0;
        return costAsc ? leftCost - rightCost : rightCost - leftCost;
      }),
    [aggregated, costAsc],
  );
  const { totals } = aggregated;

  return (
    <>
      {warnings.length > 0 && (
        <Surface className={styles.warnings} role="status" variant="glass">
          {warnings.map((warning) => (
            <div className={styles.warningRow} key={warning.provider}>
              <Icon icon={TriangleAlert} size="sm" tone="warning" />
              <span>{warning.message}</span>
            </div>
          ))}
        </Surface>
      )}

      <div className={styles.statsRow}>
        <StatCard
          icon={Activity}
          title="LLM calls"
          value={formatNumber(totals.total_calls)}
        />
        <StatCard
          icon={Database}
          title="Tokens"
          subtitle={`${formatTokenCount(
            totals.total_prompt_tokens,
          )} prompt · ${formatTokenCount(
            totals.total_completion_tokens,
          )} completion`}
          value={formatTokenCount(totals.total_tokens)}
        />
        <StatCard
          icon={CircleDollarSign}
          title="Cost"
          tone="warning"
          value={formatCostPrecise(totals.total_cost_usd)}
        />
        <StatCard
          icon={Gauge}
          title="Success rate"
          tone="success"
          value={formatRatioPercent(
            totals.successful_calls,
            totals.total_calls,
          )}
        />
      </div>

      <UsageCharts days={aggregated.by_day} />

      <section aria-label="Usage by project" className={styles.tableSection}>
        <h3 className={styles.sectionTitle}>By Project</h3>
        <Surface className={styles.tableWrapper} variant="glass">
          <table className={styles.table}>
            <thead>
              <tr>
                <th className={styles.th}>Project</th>
                <th className={styles.th}>Calls</th>
                <th className={styles.th}>Success</th>
                <th className={styles.th}>Tokens</th>
                <th className={styles.th}>
                  <button
                    className={styles.sortButton}
                    onClick={() => setCostAsc((previous) => !previous)}
                    type="button"
                  >
                    Cost {costAsc ? "↑" : "↓"}
                  </button>
                </th>
              </tr>
            </thead>
            <tbody>
              {projectRows.map((row) => (
                <tr key={row.projectId}>
                  <td className={styles.td}>{row.slug}</td>
                  <td className={styles.td}>{formatNumber(row.total_calls)}</td>
                  <td className={styles.td}>
                    {formatRatioPercent(row.successful_calls, row.total_calls)}
                  </td>
                  <td className={styles.td}>
                    {formatTokenCount(row.total_tokens)}
                  </td>
                  <td className={styles.td}>
                    {formatCostPrecise(row.total_cost_usd)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Surface>
      </section>

      <section aria-label="Usage by model" className={styles.tableSection}>
        <h3 className={styles.sectionTitle}>By Model</h3>
        <Surface className={styles.tableWrapper} variant="glass">
          <table className={styles.table}>
            <thead>
              <tr>
                <th className={styles.th}>Model</th>
                <th className={styles.th}>Provider</th>
                <th className={styles.th}>Calls</th>
                <th className={styles.th}>Success</th>
                <th className={styles.th}>Tokens</th>
                <th className={styles.th}>Cost</th>
              </tr>
            </thead>
            <tbody>
              {aggregated.by_model.map((model) => (
                <tr key={model.model_id}>
                  <td className={styles.td}>{model.model}</td>
                  <td className={styles.td}>{model.provider}</td>
                  <td className={styles.td}>
                    {formatNumber(model.total_calls)}
                  </td>
                  <td className={styles.td}>
                    {formatRatioPercent(
                      model.successful_calls,
                      model.total_calls,
                    )}
                  </td>
                  <td className={styles.td}>
                    {formatTokenCount(model.total_tokens)}
                  </td>
                  <td className={styles.td}>
                    {formatCostPrecise(model.total_cost_usd)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Surface>
      </section>
    </>
  );
}

export function UsagePage() {
  const config = useAppSelector(selectConfig);
  const daemonBase = resolveDaemonBaseUrl(config);
  const [preset, setPreset] = useState<RangePreset>("30d");
  const {
    data: workers,
    isLoading: workersLoading,
    isError: workersError,
    refetch,
  } = useListProjectsQuery(undefined);

  const readyWorkers = useMemo(
    () => (workers ?? []).filter(isReadyWorker),
    [workers],
  );
  const stoppedWorkers = useMemo(
    () => (workers ?? []).filter((worker) => !isReadyWorker(worker)),
    [workers],
  );

  const from = daysAgoIsoDate(RANGE_DAYS[preset] - 1);
  const to = todayIsoDate();

  const [usage, setUsage] = useState<UsageFetchState>({ state: "loading" });
  useEffect(() => {
    let active = true;
    setUsage({ state: "loading" });
    void fetchProjectSummaries(
      daemonBase,
      readyWorkers,
      `?from=${from}&to=${to}`,
    ).then((result) => {
      if (active) setUsage({ state: "ready", ...result });
    });
    return () => {
      active = false;
    };
  }, [daemonBase, readyWorkers, from, to]);

  const aggregated = useMemo(
    () => (usage.state === "ready" ? aggregateUsage(usage.inputs) : null),
    [usage],
  );
  const probeProjectId =
    usage.state === "ready" && usage.inputs.length > 0
      ? usage.inputs[0].projectId
      : null;

  const [warnings, setWarnings] = useState<TokenPlanWarning[]>([]);
  useEffect(() => {
    if (
      !aggregated ||
      probeProjectId === null ||
      aggregated.used_providers.length === 0
    ) {
      setWarnings([]);
      return;
    }
    let active = true;
    const providers = aggregated.used_providers.slice(0, MAX_PROVIDER_PROBES);
    void Promise.all(
      providers.map(async (provider) => {
        try {
          return tokenPlanWarning(
            provider,
            await fetchJsonWithTimeout(
              projectApiUrl(
                daemonBase,
                probeProjectId,
                `/providers/${encodeURIComponent(provider)}/account-info`,
              ),
            ),
          );
        } catch {
          return null;
        }
      }),
    ).then((results) => {
      if (active) {
        setWarnings(
          results.filter(
            (warning): warning is TokenPlanWarning => warning !== null,
          ),
        );
      }
    });
    return () => {
      active = false;
    };
  }, [aggregated, daemonBase, probeProjectId]);

  const notCountedWorkers = useMemo(() => {
    if (usage.state !== "ready") return [];
    const unavailable = new Set(usage.unavailableSlugs);
    return [
      ...stoppedWorkers.map((worker) => ({
        worker,
        reason: "not counted (worker stopped)",
        wakeable: true,
      })),
      ...readyWorkers
        .filter((worker) => unavailable.has(worker.slug))
        .map((worker) => ({
          worker,
          reason: "not counted (stats unavailable)",
          wakeable: false,
        })),
    ];
  }, [usage, stoppedWorkers, readyWorkers]);

  let content;
  if (workersLoading || usage.state === "loading") {
    content = <LoadingState label="Loading usage" variant="full" />;
  } else if (workersError) {
    content = (
      <EmptyState
        description="Could not reach the daemon to load usage data."
        icon={ChartNoAxesCombined}
        title="Usage unavailable"
        variant="full"
      />
    );
  } else if (aggregated && aggregated.totals.total_calls > 0) {
    content = <UsageContent aggregated={aggregated} warnings={warnings} />;
  } else {
    content = (
      <EmptyState
        description="Start chatting in a project to see cross-project LLM usage."
        icon={ChartNoAxesCombined}
        title="No LLM calls recorded yet"
      />
    );
  }

  return (
    <section aria-labelledby="usage-heading" className={styles.page}>
      <header className={styles.pageHeader}>
        <h2 className={styles.title} id="usage-heading">
          Usage
        </h2>
        <SegmentedControl
          aria-label="Usage date range"
          onValueChange={(value) => setPreset(value as RangePreset)}
          options={RANGE_OPTIONS}
          size="sm"
          value={preset}
        />
      </header>
      {content}
      {notCountedWorkers.length > 0 && (
        <section aria-label="Projects not counted" className={styles.stopped}>
          <h3 className={styles.sectionTitle}>Not counted</h3>
          <ul className={styles.stoppedList}>
            {notCountedWorkers.map(({ worker, reason, wakeable }) => (
              <NotCountedRow
                key={worker.project_id}
                onWoke={wakeable ? () => void refetch() : undefined}
                reason={reason}
                worker={worker}
              />
            ))}
          </ul>
        </section>
      )}
    </section>
  );
}
