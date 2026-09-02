import React from "react";
import {
  Badge,
  Card,
  Icon,
  ProviderQuota,
  Surface,
  type ProviderQuotaTone,
} from "../../../components/ui";
import {
  Activity,
  CalendarClock,
  Coins,
  Database,
  Gauge,
  ShieldCheck,
  Zap,
} from "lucide-react";
import { useGetStatsSummaryQuery } from "../../../services/refact/stats";
import {
  useGetProviderQuotasQuery,
  type ProviderQuotaFact,
  type ProviderQuotaSnapshot,
  type ProviderQuotaWindow,
} from "../../../services/refact/providers";
import { Spinner } from "../../../components/Spinner";
import { ErrorCallout } from "../../../components/Callout";
import { StatCard } from "../components/StatCard";
import { StatSection } from "../components/StatSection";
import { useIsDocumentVisible } from "../../../hooks/useIsDocumentVisible";
import {
  formatCostDisplay,
  formatCostPrecise,
  formatDuration,
  formatDurationLong,
  formatNumber,
  formatPercent,
  formatThroughput,
  formatTokenCount,
} from "../utils/formatters";
import { dateRangeToApiArgs } from "../utils/dateRange";
import {
  formatLimitWindowSeconds,
  formatQuotaMeta,
  formatResetAfterSeconds,
  formatResetAt,
} from "../../../utils/providerQuota";
import type { DateRange } from "../types";
import styles from "./OverviewTab.module.css";

const formatQuotaNumber = (value: number): string =>
  value.toLocaleString(undefined, { maximumFractionDigits: 2 });

const formatFactValue = (fact: ProviderQuotaFact): string => {
  const value =
    fact.value == null
      ? "Not reported"
      : typeof fact.value === "boolean"
        ? fact.value
          ? "Yes"
          : "No"
        : typeof fact.value === "number"
          ? formatQuotaNumber(fact.value)
          : fact.value;
  return fact.unit ? `${value} ${fact.unit}` : value;
};

const formatWindowValue = (window: ProviderQuotaWindow): string => {
  if (window.used != null && window.limit != null) {
    return `${formatQuotaNumber(window.used)} / ${formatQuotaNumber(
      window.limit,
    )}`;
  }
  if (window.remaining != null) {
    return `${formatQuotaNumber(window.remaining)} remaining`;
  }
  if (window.used_percent != null) {
    return `${formatQuotaNumber(window.used_percent)}% used`;
  }
  return window.status ?? "Not reported";
};

const windowTone = (window: ProviderQuotaWindow): ProviderQuotaTone => {
  if (window.status?.toLowerCase().includes("limit")) return "danger";
  if (window.used_percent == null) return "muted";
  if (window.used_percent >= 90) return "danger";
  if (window.used_percent >= 70) return "warning";
  return "success";
};

const formatFetchedAt = (value: string): string => {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value || "Unknown"
    : date.toLocaleString();
};

const getPlan = (snapshot: ProviderQuotaSnapshot): string | null => {
  if (snapshot.plan != null) return snapshot.plan;
  const plan = snapshot.facts.find((fact) =>
    ["plan", "plan_type", "tier"].includes(fact.id.toLowerCase()),
  );
  return plan?.value == null ? null : String(plan.value);
};

const ProviderQuotaCard: React.FC<{ snapshot: ProviderQuotaSnapshot }> = ({
  snapshot,
}) => {
  const plan = getPlan(snapshot);
  const fetchedAt = formatFetchedAt(snapshot.fetched_at);
  const stateTone: ProviderQuotaTone = !snapshot.available
    ? "danger"
    : snapshot.stale
      ? "warning"
      : "success";

  return (
    <Card animated="rise" className={styles.quotaCard}>
      <div className={styles.quotaHeader}>
        <div className={styles.quotaLabel}>
          <span className={styles.quotaName}>{snapshot.provider_name}</span>
          {plan && <Badge tone="accent">{plan}</Badge>}
          {!snapshot.available && <Badge tone="danger">Unavailable</Badge>}
          {snapshot.stale && <Badge tone="warning">Stale</Badge>}
        </div>
        <span className={styles.quotaProvider}>{snapshot.base_provider}</span>
      </div>

      <div className={styles.quotaSummary}>
        <span>Source: {snapshot.source || "Unknown"}</span>
        <span>Fetched: {fetchedAt}</span>
      </div>

      {snapshot.error && (
        <ProviderQuota
          label="Provider status"
          value="Error"
          meta={snapshot.error}
          tone="danger"
        />
      )}
      {!snapshot.error && !snapshot.available && (
        <ProviderQuota
          label="Provider status"
          value="Unavailable"
          meta="Quota data is not available for this provider."
          tone={stateTone}
        />
      )}

      {snapshot.windows.map((window) => {
        const windowLength = formatLimitWindowSeconds(window.window_seconds);
        const meta = formatQuotaMeta([
          windowLength ? `Window ${windowLength}` : null,
          formatResetAt(window.reset_at),
          formatResetAfterSeconds(window.reset_after_seconds),
          window.status,
        ]);
        return (
          <ProviderQuota
            key={window.id}
            label={window.label}
            value={formatWindowValue(window)}
            meta={meta || undefined}
            usedPercent={window.used_percent ?? undefined}
            tone={snapshot.stale ? "warning" : windowTone(window)}
          />
        );
      })}

      {snapshot.facts.map((fact) => (
        <ProviderQuota
          key={fact.id}
          label={fact.label}
          value={formatFactValue(fact)}
          tone={snapshot.stale ? "warning" : "muted"}
        />
      ))}

      {snapshot.available &&
        !snapshot.error &&
        snapshot.windows.length === 0 &&
        snapshot.facts.length === 0 && (
          <ProviderQuota
            label="Quota"
            value="No limits reported"
            tone={stateTone}
          />
        )}
    </Card>
  );
};

const ProviderQuotaSection: React.FC = () => {
  const visible = useIsDocumentVisible();
  const { data, isLoading, isError } = useGetProviderQuotasQuery(undefined, {
    pollingInterval: visible ? 5 * 60_000 : 0,
  });
  const snapshots = (data?.quotas ?? []).filter(
    (snapshot) => snapshot.available,
  );

  return (
    <section className={styles.quotaSection}>
      <h3 className={styles.sectionTitle}>
        <Icon icon={Gauge} size="md" tone="accent" />
        Provider Quotas
      </h3>
      {isLoading && <Spinner spinning />}
      {isError && (
        <ErrorCallout>Failed to load provider quota snapshots</ErrorCallout>
      )}
      {!isLoading && !isError && snapshots.length === 0 && (
        <p className={styles.emptyText}>
          No provider quota snapshots reported.
        </p>
      )}
      {snapshots.length > 0 && (
        <div className={`${styles.quotaGrid} rf-stagger`}>
          {snapshots.map((snapshot) => (
            <ProviderQuotaCard
              key={snapshot.provider_name}
              snapshot={snapshot}
            />
          ))}
        </div>
      )}
    </section>
  );
};

type Props = { dateRange: DateRange };

const formatErrorLabel = (key: string): string =>
  key
    .replace(/_/g, " ")
    .split(" ")
    .filter(Boolean)
    .map((word) => `${word.charAt(0).toUpperCase()}${word.slice(1)}`)
    .join(" ");

export const OverviewTab: React.FC<Props> = ({ dateRange }) => {
  const { data, isLoading, isError } = useGetStatsSummaryQuery(
    dateRangeToApiArgs(dateRange),
  );

  if (isLoading) return <Spinner spinning />;
  if (isError) return <ErrorCallout>Failed to load stats</ErrorCallout>;

  const t = data?.totals;
  const hasStats = !!(t && t.total_calls > 0);

  const activeDays = t?.active_days ?? 0;
  const callsPerDay =
    t && activeDays > 0 ? Math.round(t.total_calls / activeDays) : 0;
  const avgTokensPerCall =
    t && t.total_calls > 0 ? Math.round(t.total_tokens / t.total_calls) : 0;
  const avgTokensPerConversation =
    t && t.total_conversations > 0
      ? Math.round(t.total_tokens / t.total_conversations)
      : 0;
  const completionShare =
    t && t.total_tokens > 0
      ? (t.total_completion_tokens / t.total_tokens) * 100
      : 0;
  const successRate =
    t && t.total_calls > 0 ? (t.successful_calls / t.total_calls) * 100 : 0;
  const successTone =
    successRate >= 95 ? "success" : successRate >= 80 ? "warning" : "danger";
  const costPerConversation =
    t && t.total_conversations > 0 && t.total_cost_usd
      ? t.total_cost_usd / t.total_conversations
      : t?.total_cost_usd == null
        ? null
        : 0;
  const costPerMillionTokens =
    t && t.total_tokens > 0 && t.total_cost_usd
      ? (t.total_cost_usd / t.total_tokens) * 1_000_000
      : t?.total_cost_usd == null
        ? null
        : 0;
  const costPerDay =
    t && activeDays > 0 && t.total_cost_usd
      ? t.total_cost_usd / activeDays
      : t?.total_cost_usd == null
        ? null
        : 0;
  const errorCategories = data?.errors?.by_category ?? [];
  const topError = errorCategories.at(0);
  const topErrorLabel = topError ? formatErrorLabel(topError.key) : "None";
  // Input tokens are split into uncached prompt + cache reads + cache writes;
  // the hit rate is the share of all input tokens served from cache (0-100%).
  const totalInputTokens = t
    ? t.total_prompt_tokens +
      t.total_cache_read_tokens +
      t.total_cache_creation_tokens
    : 0;
  const cacheHitRate =
    t && totalInputTokens > 0
      ? (t.total_cache_read_tokens / totalInputTokens) * 100
      : 0;
  const cacheReuse =
    t && t.total_cache_creation_tokens > 0
      ? t.total_cache_read_tokens / t.total_cache_creation_tokens
      : 0;

  const topConversations = data?.top_conversations ?? [];

  return (
    <div className={styles.root}>
      <ProviderQuotaSection />
      {!hasStats && (
        <p className={styles.emptyText}>
          No usage data yet. Start chatting to see stats!
        </p>
      )}
      {hasStats && (
        <>
          <div className={styles.groups}>
            <StatSection title="Volume & Activity" icon={Activity}>
              <StatCard
                title="Total Calls"
                value={formatNumber(t.total_calls)}
              />
              <StatCard
                title="Conversations"
                value={formatNumber(t.total_conversations)}
              />
              <StatCard
                title="Messages Sent"
                value={formatNumber(t.total_messages_sent)}
              />
              <StatCard
                title="Tasks"
                value={formatNumber(t.total_tasks ?? 0)}
              />
              <StatCard
                title="Agents"
                value={formatNumber(t.total_agents ?? 0)}
              />
              <StatCard
                title="Active Days"
                value={formatNumber(t.active_days ?? 0)}
              />
              <StatCard
                title="Calls / Day"
                value={formatNumber(callsPerDay)}
                subtitle={activeDays > 0 ? undefined : "—"}
              />
            </StatSection>

            <StatSection title="Tokens" icon={Zap}>
              <StatCard
                title="Total Tokens"
                value={formatTokenCount(t.total_tokens)}
                subtitle={`${formatTokenCount(
                  t.total_prompt_tokens,
                )} read + ${formatTokenCount(
                  t.total_completion_tokens,
                )} written`}
              />
              <StatCard
                title="Prompt (read)"
                value={formatTokenCount(t.total_prompt_tokens)}
              />
              <StatCard
                title="Completion (written)"
                value={formatTokenCount(t.total_completion_tokens)}
                tone="accent"
              />
              <StatCard
                title="Avg Tokens / Call"
                value={formatTokenCount(avgTokensPerCall)}
              />
              <StatCard
                title="Avg Tokens / Conversation"
                value={formatTokenCount(avgTokensPerConversation)}
              />
              <StatCard
                title="Completion Share"
                value={formatPercent(completionShare)}
              />
            </StatSection>

            <StatSection title="Cost" icon={Coins}>
              <StatCard
                title="Total Cost"
                value={formatCostDisplay(t.total_cost_usd)}
                tone="warning"
              />
              <StatCard
                title="Cost / Conversation"
                value={formatCostPrecise(costPerConversation)}
              />
              <StatCard
                title="Cost / 1M Tokens"
                value={formatCostPrecise(costPerMillionTokens)}
              />
              <StatCard
                title="Cost / Day"
                value={formatCostPrecise(costPerDay)}
              />
            </StatSection>

            <StatSection title="Performance" icon={Gauge}>
              <StatCard
                title="Avg Duration"
                value={formatDuration(t.avg_duration_ms)}
                subtitle="per LLM call"
              />
              <StatCard
                title="Total Compute"
                value={formatDurationLong(t.total_duration_ms)}
              />
              <StatCard
                title="Throughput"
                value={formatThroughput(
                  t.total_completion_tokens,
                  t.total_duration_ms,
                )}
                subtitle="completion tokens/sec"
              />
            </StatSection>

            <StatSection title="Reliability" icon={ShieldCheck}>
              <StatCard
                title="Success Rate"
                value={formatPercent(successRate)}
                tone={successTone}
                subtitle={`${formatNumber(
                  t.successful_calls,
                )} of ${formatNumber(t.total_calls)} succeeded`}
              />
              <StatCard
                title="Failed Calls"
                value={formatNumber(t.failed_calls)}
                tone={t.failed_calls > 0 ? "danger" : "muted"}
              />
              <StatCard
                title="Retried Calls"
                value={formatNumber(t.retried_calls ?? 0)}
              />
              <StatCard
                title="Top Error"
                value={topErrorLabel}
                subtitle={topError ? formatNumber(topError.count) : undefined}
                tone={topError ? "danger" : "success"}
              />
            </StatSection>

            <StatSection title="Cache" icon={Database}>
              <StatCard
                title="Cache Read"
                value={formatTokenCount(t.total_cache_read_tokens)}
                tone="success"
              />
              <StatCard
                title="Cache Created"
                value={formatTokenCount(t.total_cache_creation_tokens)}
              />
              <StatCard
                title="Cache Hit Rate"
                value={formatPercent(cacheHitRate)}
                subtitle="of input tokens served from cache"
                tone="success"
              />
              <StatCard
                title="Cache Reuse"
                value={cacheReuse > 0 ? `${cacheReuse.toFixed(1)}×` : "—"}
                subtitle="tokens read per token cached"
              />
            </StatSection>
          </div>

          {topConversations.length > 0 && (
            <section className={styles.root}>
              <h3 className={styles.sectionTitle}>
                <Icon icon={CalendarClock} size="md" tone="accent" />
                Top Conversations by Token Usage
              </h3>
              <Surface
                className={`${styles.tableWrapper} rf-enter-rise`}
                variant="plain"
              >
                <table className={styles.table}>
                  <thead>
                    <tr>
                      <th className={styles.th}>Chat ID</th>
                      <th className={styles.th}>Model</th>
                      <th className={styles.th}>Calls</th>
                      <th className={styles.th}>Tokens</th>
                      <th className={styles.th}>Cost</th>
                    </tr>
                  </thead>
                  <tbody>
                    {topConversations.map((c) => (
                      <tr key={c.chat_id}>
                        <td className={styles.td}>
                          <span className={styles.chatId} title={c.chat_id}>
                            {c.chat_id.slice(0, 8)}
                          </span>
                        </td>
                        <td className={styles.td}>{c.model_id}</td>
                        <td className={styles.td}>{c.total_calls}</td>
                        <td className={styles.td}>
                          {formatTokenCount(c.total_tokens)}
                        </td>
                        <td className={styles.td}>
                          {formatCostDisplay(c.total_cost_usd)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </Surface>
            </section>
          )}
        </>
      )}
    </div>
  );
};
