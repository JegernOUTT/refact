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
  ProviderQuota,
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
import type {
  ProviderQuotaFact,
  ProviderQuotaListResponse,
  ProviderQuotaSnapshot,
  ProviderQuotaWindow,
} from "../../../services/refact/providers";
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
  formatLimitWindowSeconds,
  formatQuotaMeta,
  formatResetAfterSeconds,
  formatResetAt,
  formatUsagePercent,
} from "../../../utils/providerQuota";
import {
  aggregateUsage,
  type AggregatedUsage,
  type ProjectUsageInput,
} from "./aggregate";
import { UsageCharts } from "./UsageCharts";
import styles from "./Usage.module.css";

const REQUEST_TIMEOUT_MS = 5_000;
const MAX_CONCURRENT_REQUESTS = 3;
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
  key: string;
  message: string;
};

type ProjectQuotaResult = {
  projectId: string;
  slug: string;
  state: "ready" | "error";
  quotas: ProviderQuotaSnapshot[];
};

type QuotaFetchState =
  | { state: "loading" }
  | { state: "ready"; projects: ProjectQuotaResult[] };

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

async function collectBounded<T, R>(
  items: T[],
  collect: (item: T) => Promise<R>,
  concurrency = MAX_CONCURRENT_REQUESTS,
): Promise<R[]> {
  const results = new Array<R>(items.length);
  let nextIndex = 0;
  async function run() {
    while (nextIndex < items.length) {
      const index = nextIndex;
      nextIndex += 1;
      results[index] = await collect(items[index]);
    }
  }
  await Promise.all(
    Array.from({ length: Math.min(concurrency, items.length) }, run),
  );
  return results;
}

function isQuotaListResponse(data: unknown): data is ProviderQuotaListResponse {
  if (!data || typeof data !== "object") return false;
  const quotas = (data as { quotas?: unknown }).quotas;
  return Array.isArray(quotas) && quotas.every(isQuotaSnapshot);
}

function isNullableFiniteNumber(value: unknown): boolean {
  return (
    value === null || (typeof value === "number" && Number.isFinite(value))
  );
}

function isOptionalNullableString(
  record: Record<string, unknown>,
  key: string,
): boolean {
  return (
    !(key in record) || record[key] == null || typeof record[key] === "string"
  );
}

function isQuotaWindow(value: unknown): value is ProviderQuotaWindow {
  if (!value || typeof value !== "object") return false;
  const window = value as Record<string, unknown>;
  return (
    typeof window.id === "string" &&
    typeof window.label === "string" &&
    isNullableFiniteNumber(window.used_percent) &&
    [
      "limit",
      "used",
      "remaining",
      "reset_after_seconds",
      "window_seconds",
    ].every(
      (key) =>
        !(key in window) ||
        window[key] === undefined ||
        isNullableFiniteNumber(window[key]),
    ) &&
    isOptionalNullableString(window, "reset_at") &&
    isOptionalNullableString(window, "status")
  );
}

function isQuotaFact(value: unknown): value is ProviderQuotaFact {
  if (!value || typeof value !== "object") return false;
  const fact = value as Record<string, unknown>;
  return (
    typeof fact.id === "string" &&
    typeof fact.label === "string" &&
    "value" in fact &&
    (fact.value === null ||
      typeof fact.value === "string" ||
      typeof fact.value === "boolean" ||
      (typeof fact.value === "number" && Number.isFinite(fact.value))) &&
    isOptionalNullableString(fact, "unit")
  );
}

function isQuotaSnapshot(value: unknown): value is ProviderQuotaSnapshot {
  if (!value || typeof value !== "object") return false;
  const snapshot = value as Record<string, unknown>;
  return (
    typeof snapshot.provider_name === "string" &&
    typeof snapshot.base_provider === "string" &&
    typeof snapshot.source === "string" &&
    typeof snapshot.available === "boolean" &&
    typeof snapshot.fetched_at === "string" &&
    typeof snapshot.stale === "boolean" &&
    isOptionalNullableString(snapshot, "plan") &&
    isOptionalNullableString(snapshot, "error") &&
    Array.isArray(snapshot.windows) &&
    snapshot.windows.every(isQuotaWindow) &&
    Array.isArray(snapshot.facts) &&
    snapshot.facts.every(isQuotaFact)
  );
}

async function fetchProjectQuotas(
  daemonBase: string,
  workers: DaemonWorker[],
): Promise<ProjectQuotaResult[]> {
  return collectBounded(workers, async (worker) => {
    try {
      const data = await fetchJsonWithTimeout(
        projectApiUrl(daemonBase, worker.project_id, "/providers/quotas"),
      );
      if (!isQuotaListResponse(data)) throw new Error("Invalid response");
      return {
        projectId: worker.project_id,
        slug: worker.slug,
        state: "ready" as const,
        quotas: data.quotas,
      };
    } catch {
      return {
        projectId: worker.project_id,
        slug: worker.slug,
        state: "error" as const,
        quotas: [],
      };
    }
  });
}

function quotaWarning(
  project: ProjectQuotaResult,
  quota: ProviderQuotaSnapshot,
  window: ProviderQuotaWindow,
): TokenPlanWarning | null {
  const identity = `${quota.provider_name} on ${project.slug}`;
  const status = window.status?.toLocaleLowerCase() ?? "";
  const exhausted =
    window.remaining !== undefined &&
    window.remaining !== null &&
    window.remaining <= 0;
  if (
    exhausted ||
    (window.used_percent !== null && window.used_percent >= 100) ||
    status.includes("limit") ||
    status.includes("exhaust")
  ) {
    return {
      key: `${project.projectId}:${quota.provider_name}:${window.id}`,
      message: `${identity}: ${window.label} quota exhausted`,
    };
  }
  const lowByPercent =
    window.used_percent !== null && window.used_percent >= 90;
  const lowByRemaining =
    window.limit !== undefined &&
    window.limit !== null &&
    window.limit > 0 &&
    window.remaining !== undefined &&
    window.remaining !== null &&
    window.remaining <= window.limit * LOW_PLAN_REMAINING_RATIO;
  if (lowByPercent || lowByRemaining) {
    const remaining =
      window.remaining !== undefined && window.remaining !== null
        ? ` (${formatNumber(window.remaining)} remaining)`
        : "";
    return {
      key: `${project.projectId}:${quota.provider_name}:${window.id}`,
      message: `${identity}: ${window.label} quota is nearly exhausted${remaining}`,
    };
  }
  return null;
}

function quotaWarnings(projects: ProjectQuotaResult[]): TokenPlanWarning[] {
  return projects.flatMap((project) =>
    project.quotas.flatMap((quota) => {
      const windowWarnings = quota.windows
        .map((window) => quotaWarning(project, quota, window))
        .filter((warning): warning is TokenPlanWarning => warning !== null);
      const numericFacts = new Map(
        quota.facts
          .filter(
            (fact): fact is ProviderQuotaFact & { value: number } =>
              typeof fact.value === "number" && Number.isFinite(fact.value),
          )
          .map((fact) => [fact.id.toLocaleLowerCase(), fact.value]),
      );
      const remaining = [...numericFacts].find(([id]) =>
        id.includes("remaining"),
      )?.[1];
      const limit = [...numericFacts].find(
        ([id]) => id.includes("limit") || id.includes("max_budget"),
      )?.[1];
      const exhaustedFact = quota.facts.some((fact) => {
        const id = fact.id.toLocaleLowerCase();
        return (
          ((id.includes("limit_reached") || id.includes("exhausted")) &&
            fact.value === true) ||
          (id.includes("status") &&
            typeof fact.value === "string" &&
            /limit|exhaust/i.test(fact.value))
        );
      });
      const lowFact =
        remaining !== undefined &&
        limit !== undefined &&
        limit > 0 &&
        remaining <= limit * LOW_PLAN_REMAINING_RATIO;
      if (exhaustedFact || (remaining !== undefined && remaining <= 0)) {
        windowWarnings.push({
          key: `${project.projectId}:${quota.provider_name}:facts`,
          message: `${quota.provider_name} on ${project.slug}: quota exhausted`,
        });
      } else if (lowFact) {
        windowWarnings.push({
          key: `${project.projectId}:${quota.provider_name}:facts`,
          message: `${quota.provider_name} on ${
            project.slug
          }: quota is nearly exhausted (${formatNumber(remaining)} remaining)`,
        });
      }
      return windowWarnings;
    }),
  );
}

function quotaNumber(value: number): string {
  return value.toLocaleString(undefined, { maximumFractionDigits: 2 });
}

function windowValue(window: ProviderQuotaWindow): string {
  if (window.remaining != null && window.limit != null) {
    return `${quotaNumber(window.remaining)} / ${quotaNumber(
      window.limit,
    )} remaining`;
  }
  if (window.used != null && window.limit != null) {
    return `${quotaNumber(window.used)} / ${quotaNumber(window.limit)}`;
  }
  if (window.used_percent != null)
    return formatUsagePercent(window.used_percent);
  if (window.remaining != null)
    return `${quotaNumber(window.remaining)} remaining`;
  return "Usage reported";
}

function factValue(fact: ProviderQuotaFact): string {
  if (fact.value === null) return "Not reported";
  const value =
    typeof fact.value === "boolean"
      ? fact.value
        ? "Yes"
        : "No"
      : typeof fact.value === "number"
        ? quotaNumber(fact.value)
        : fact.value;
  return fact.unit ? `${value} ${fact.unit}` : String(value);
}

function quotaMeta(window: ProviderQuotaWindow): string | undefined {
  const windowDuration = formatLimitWindowSeconds(window.window_seconds);
  return (
    formatQuotaMeta([
      window.used_percent === null
        ? null
        : formatUsagePercent(window.used_percent),
      windowDuration ? `Window ${windowDuration}` : null,
      formatResetAt(window.reset_at),
      formatResetAfterSeconds(window.reset_after_seconds),
      window.status ?? null,
    ]) || undefined
  );
}

function quotaTone(quota: ProviderQuotaSnapshot, usedPercent?: number) {
  if (!quota.available || quota.error) return "danger" as const;
  if (quota.stale) return "muted" as const;
  if (usedPercent !== undefined && usedPercent >= 90) return "danger" as const;
  if (usedPercent !== undefined && usedPercent >= 70) return "warning" as const;
  return "success" as const;
}

function QuotaSection({ quotaState }: { quotaState: QuotaFetchState }) {
  return (
    <section
      aria-label="Provider quotas across projects"
      className={styles.quotaSection}
    >
      <h3 className={styles.sectionTitle}>
        <Icon icon={Gauge} size="sm" tone="accent" />
        Provider quotas across projects
      </h3>
      {quotaState.state === "loading" ? (
        <LoadingState label="Loading provider quotas" />
      ) : (
        <div className={styles.quotaGrid}>
          {quotaState.projects.map((project) => {
            if (project.state === "error") {
              return (
                <ProviderQuota
                  key={project.projectId}
                  label={`${project.slug} · provider quotas`}
                  meta="Could not load quotas from this project"
                  tone="danger"
                  value="Error"
                />
              );
            }
            if (project.quotas.length === 0) {
              return (
                <ProviderQuota
                  key={project.projectId}
                  label={`${project.slug} · provider quotas`}
                  meta="No provider reported quota information"
                  tone="muted"
                  value="Unavailable"
                />
              );
            }
            return project.quotas.flatMap((quota) => {
              const identity = `${project.slug} · ${quota.provider_name}`;
              const badge = quota.stale
                ? "Stale"
                : quota.available
                  ? `${quota.base_provider} · Available`
                  : "Unavailable";
              if (
                !quota.available ||
                quota.windows.length + quota.facts.length === 0
              ) {
                return [
                  <ProviderQuota
                    badge={badge}
                    key={`${project.projectId}:${quota.provider_name}`}
                    label={identity}
                    meta={quota.error ?? "Quota information is unavailable"}
                    tone={quotaTone(quota)}
                    value={quota.error ? "Error" : "Unavailable"}
                  />,
                ];
              }
              return [
                ...quota.windows.map((window) => (
                  <ProviderQuota
                    badge={badge}
                    key={`${project.projectId}:${quota.provider_name}:window:${window.id}`}
                    label={`${identity} · ${window.label}`}
                    meta={quotaMeta(window)}
                    tone={quotaTone(quota, window.used_percent ?? undefined)}
                    usedPercent={window.used_percent ?? undefined}
                    value={windowValue(window)}
                  />
                )),
                ...quota.facts.map((fact) => (
                  <ProviderQuota
                    badge={badge}
                    key={`${project.projectId}:${quota.provider_name}:fact:${fact.id}`}
                    label={`${identity} · ${fact.label}`}
                    meta={quota.error ?? undefined}
                    tone={quotaTone(quota)}
                    value={factValue(fact)}
                  />
                )),
              ];
            });
          })}
        </div>
      )}
    </section>
  );
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
            <div className={styles.warningRow} key={warning.key}>
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

  const [quotaState, setQuotaState] = useState<QuotaFetchState>({
    state: "loading",
  });
  useEffect(() => {
    let active = true;
    setQuotaState({ state: "loading" });
    void fetchProjectQuotas(daemonBase, readyWorkers).then((projects) => {
      if (active) setQuotaState({ state: "ready", projects });
    });
    return () => {
      active = false;
    };
  }, [daemonBase, readyWorkers]);
  const warnings = useMemo(
    () =>
      quotaState.state === "ready" ? quotaWarnings(quotaState.projects) : [],
    [quotaState],
  );

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
      {!workersLoading && !workersError && readyWorkers.length > 0 && (
        <QuotaSection quotaState={quotaState} />
      )}
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
