import React, { useMemo } from "react";
import { Badge, Card, Icon, StatusDot, Surface } from "../../../components/ui";
import {
  Activity,
  Bot,
  CalendarClock,
  Coins,
  Database,
  Gauge,
  ServerCog,
  ShieldCheck,
  Zap,
} from "lucide-react";
import { useGetStatsSummaryQuery } from "../../../services/refact/stats";
import {
  useGetClaudeCodeUsageQuery,
  useGetOpenAICodexUsageQuery,
  useGetOpenCodeUsageQuery,
  type OpenCodeUsageData,
} from "../../../services/refact/providers";
import { useGetConfiguredProvidersQuery } from "../../../hooks";
import { Spinner } from "../../../components/Spinner";
import { ErrorCallout } from "../../../components/Callout";
import { StatCard } from "../components/StatCard";
import { StatSection } from "../components/StatSection";
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
  clampPercent,
  formatClaudeExtraUsage,
  formatCodexCreditsDetails,
  formatCodexCreditsSummary,
  formatLimitWindowSeconds,
  formatQuotaMeta,
  formatResetAt,
  formatUsagePercent,
} from "../../../utils/providerQuota";
import type { DateRange } from "../types";
import styles from "./OverviewTab.module.css";

type UsageTone = "danger" | "warning" | "success";

const usageTone = (pct: number): UsageTone => {
  if (pct >= 90) return "danger";
  if (pct >= 70) return "warning";
  return "success";
};

const usageStatus: Record<UsageTone, "error" | "warning" | "success"> = {
  danger: "error",
  warning: "warning",
  success: "success",
};

const UsageBar: React.FC<{ pct: number }> = ({ pct }) => {
  const clamped = clampPercent(pct);
  const tone = usageTone(clamped);
  const color =
    tone === "danger"
      ? "var(--rf-color-danger)"
      : tone === "warning"
        ? "var(--rf-color-warning)"
        : "var(--rf-color-success)";

  return (
    <div
      aria-label={`${Math.round(clamped)}% used`}
      className={styles.usageBar}
      role="meter"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={clamped}
    >
      <div
        className={styles.usageBarFill}
        style={
          {
            "--usage-pct": `${clamped}%`,
            "--usage-color": color,
          } as React.CSSProperties
        }
      />
    </div>
  );
};

const QuotaLine: React.FC<{
  label: string;
  pct: number;
  resetAt?: string | null;
  limitReached?: boolean;
  windowSeconds?: number | null;
}> = ({ label, pct, resetAt, limitReached = false, windowSeconds }) => {
  const clamped = clampPercent(pct);
  const windowText = formatLimitWindowSeconds(windowSeconds);
  const meta = formatQuotaMeta([
    formatUsagePercent(clamped),
    windowText ? `Window ${windowText}` : null,
    formatResetAt(resetAt),
  ]);

  return (
    <div className={styles.quotaLine}>
      <div className={styles.quotaLabel}>
        <StatusDot
          status={limitReached ? "error" : usageStatus[usageTone(clamped)]}
          pulse={limitReached}
        />
        <span className={styles.quotaMeta}>{label}</span>
        {limitReached && <Badge tone="danger">Limit reached</Badge>}
      </div>
      <span className={styles.quotaMeta}>{meta}</span>
    </div>
  );
};

type OpenCodeUsageWindowKey = keyof Pick<
  OpenCodeUsageData,
  "rolling" | "weekly" | "monthly"
>;

const OPENCODE_USAGE_WINDOWS: {
  key: OpenCodeUsageWindowKey;
  label: string;
}[] = [
  { key: "rolling", label: "Rolling" },
  { key: "weekly", label: "Weekly" },
  { key: "monthly", label: "Monthly" },
];

function hasOpenCodeQuotaData(data: OpenCodeUsageData): boolean {
  return (
    typeof data.balance === "number" ||
    OPENCODE_USAGE_WINDOWS.some(({ key }) => Boolean(data[key]))
  );
}

const ClaudeCodeInstanceCard: React.FC<{
  providerName: string;
  displayName: string;
}> = ({ providerName, displayName }) => {
  const { data: claudeUsage } = useGetClaudeCodeUsageQuery(
    { providerName },
    { pollingInterval: 5 * 60_000 },
  );
  const data = claudeUsage?.data;
  if (!data) return null;
  if (!data.five_hour && !data.seven_day) return null;

  return (
    <Card animated="rise" className={styles.quotaCard}>
      <div className={styles.quotaHeader}>
        <div className={styles.quotaLabel}>
          <Icon icon={ServerCog} size="md" tone="accent" />
          <span className={styles.quotaName}>{displayName}</span>
        </div>
        <span className={styles.quotaProvider}>{providerName}</span>
      </div>
      {data.five_hour && (
        <div>
          <QuotaLine
            label="Session (5h)"
            pct={data.five_hour.percent_used}
            resetAt={data.five_hour.resets_at}
          />
          <UsageBar pct={data.five_hour.percent_used} />
        </div>
      )}
      {data.seven_day && (
        <div>
          <QuotaLine
            label="Weekly"
            pct={data.seven_day.percent_used}
            resetAt={data.seven_day.resets_at}
          />
          <UsageBar pct={data.seven_day.percent_used} />
        </div>
      )}
      {data.extra_usage && (
        <span className={styles.quotaExtra}>
          Extra: {formatClaudeExtraUsage(data.extra_usage)}
        </span>
      )}
    </Card>
  );
};

const OpenAICodexInstanceCard: React.FC<{
  providerName: string;
  displayName: string;
}> = ({ providerName, displayName }) => {
  const { data: codexUsage } = useGetOpenAICodexUsageQuery(
    { providerName },
    { pollingInterval: 5 * 60_000 },
  );
  const data = codexUsage?.data;
  if (!data?.rate_limit) return null;

  const creditsDetails = data.credits
    ? formatCodexCreditsDetails(data.credits)
    : null;

  return (
    <Card animated="rise" className={styles.quotaCard}>
      <div className={styles.quotaHeader}>
        <div className={styles.quotaLabel}>
          <Icon icon={Bot} size="md" tone="accent" />
          <span className={styles.quotaName}>{displayName}</span>
          {data.plan_type && <Badge tone="accent">{data.plan_type}</Badge>}
        </div>
        <span className={styles.quotaProvider}>{providerName}</span>
      </div>
      {data.rate_limit.primary_window && (
        <div>
          <QuotaLine
            label="Session"
            pct={data.rate_limit.primary_window.used_percent}
            resetAt={data.rate_limit.primary_window.reset_at}
            limitReached={data.rate_limit.limit_reached}
            windowSeconds={data.rate_limit.primary_window.limit_window_seconds}
          />
          <UsageBar pct={data.rate_limit.primary_window.used_percent} />
        </div>
      )}
      {data.rate_limit.secondary_window && (
        <div>
          <QuotaLine
            label="Secondary"
            pct={data.rate_limit.secondary_window.used_percent}
            resetAt={data.rate_limit.secondary_window.reset_at}
            windowSeconds={
              data.rate_limit.secondary_window.limit_window_seconds
            }
          />
          <UsageBar pct={data.rate_limit.secondary_window.used_percent} />
        </div>
      )}
      {data.code_review_rate_limit?.primary_window && (
        <div>
          <QuotaLine
            label="Code review"
            pct={data.code_review_rate_limit.primary_window.used_percent}
            resetAt={data.code_review_rate_limit.primary_window.reset_at}
            limitReached={data.code_review_rate_limit.limit_reached}
            windowSeconds={
              data.code_review_rate_limit.primary_window.limit_window_seconds
            }
          />
          <UsageBar
            pct={data.code_review_rate_limit.primary_window.used_percent}
          />
        </div>
      )}
      {data.code_review_rate_limit?.secondary_window && (
        <div>
          <QuotaLine
            label="Code review secondary"
            pct={data.code_review_rate_limit.secondary_window.used_percent}
            resetAt={data.code_review_rate_limit.secondary_window.reset_at}
            windowSeconds={
              data.code_review_rate_limit.secondary_window.limit_window_seconds
            }
          />
          <UsageBar
            pct={data.code_review_rate_limit.secondary_window.used_percent}
          />
        </div>
      )}
      {data.credits && (
        <span className={styles.quotaExtra}>
          Credits: {formatCodexCreditsSummary(data.credits)}
          {creditsDetails ? ` · ${creditsDetails}` : ""}
        </span>
      )}
    </Card>
  );
};

const OpenCodeInstanceCard: React.FC<{
  providerName: string;
  displayName: string;
  data: OpenCodeUsageData;
}> = ({ providerName, displayName, data }) => {
  const windows = OPENCODE_USAGE_WINDOWS.map(({ key, label }) => ({
    key,
    label,
    window: data[key],
  })).filter(
    (
      row,
    ): row is {
      key: OpenCodeUsageWindowKey;
      label: string;
      window: NonNullable<OpenCodeUsageData[OpenCodeUsageWindowKey]>;
    } => Boolean(row.window),
  );
  if (!hasOpenCodeQuotaData(data)) return null;

  return (
    <Card animated="rise" className={styles.quotaCard}>
      <div className={styles.quotaHeader}>
        <div className={styles.quotaLabel}>
          <Icon icon={Bot} size="md" tone="accent" />
          <span className={styles.quotaName}>{displayName}</span>
          {data.plan_type && <Badge tone="accent">{data.plan_type}</Badge>}
        </div>
        <span className={styles.quotaProvider}>{providerName}</span>
      </div>
      {data.workspace_id && (
        <span className={styles.quotaExtra}>
          Workspace: {data.workspace_id}
        </span>
      )}
      {typeof data.balance === "number" && (
        <span className={styles.quotaExtra}>
          Zen balance:{" "}
          {data.balance.toLocaleString(undefined, { maximumFractionDigits: 2 })}
        </span>
      )}
      {windows.map(({ key, label, window }) => (
        <div key={key}>
          <QuotaLine
            label={
              formatLimitWindowSeconds(window.limit_window_seconds) ?? label
            }
            pct={window.used_percent}
            resetAt={window.reset_at}
            limitReached={window.status === "rate-limited"}
            windowSeconds={window.limit_window_seconds}
          />
          <UsageBar pct={window.used_percent} />
        </div>
      ))}
    </Card>
  );
};

const OpenCodeProviderQuotaCard: React.FC<{
  providerName: string;
  displayName: string;
}> = ({ providerName, displayName }) => {
  const { data: openCodeUsage } = useGetOpenCodeUsageQuery(
    { providerName },
    { pollingInterval: 5 * 60_000 },
  );
  const data = openCodeUsage?.data;
  if (!data || !hasOpenCodeQuotaData(data)) return null;

  return (
    <section className={styles.root}>
      <h3 className={styles.sectionTitle}>
        <Icon icon={Gauge} size="md" tone="accent" />
        OpenCode Quota
      </h3>
      <div className={`${styles.quotaGrid} rf-stagger`}>
        <OpenCodeInstanceCard
          providerName={providerName}
          displayName={displayName}
          data={data}
        />
      </div>
    </section>
  );
};

const OpenCodeProviderQuotaSections: React.FC = () => {
  const { data: providersData } = useGetConfiguredProvidersQuery();
  const providers = useMemo(
    () => providersData?.providers ?? [],
    [providersData],
  );
  const openCodeInstances = useMemo(
    () => providers.filter((p) => p.base_provider === "opencode" && p.enabled),
    [providers],
  );

  return (
    <>
      {openCodeInstances.map((p) => (
        <OpenCodeProviderQuotaCard
          key={`opencode:${p.name}`}
          providerName={p.name}
          displayName={p.display_name}
        />
      ))}
    </>
  );
};

const ProviderQuotaSection: React.FC = () => {
  const { data: providersData } = useGetConfiguredProvidersQuery();
  const providers = useMemo(
    () => providersData?.providers ?? [],
    [providersData],
  );

  const claudeInstances = useMemo(
    () =>
      providers.filter((p) => p.base_provider === "claude_code" && p.enabled),
    [providers],
  );
  const codexInstances = useMemo(
    () =>
      providers.filter((p) => p.base_provider === "openai_codex" && p.enabled),
    [providers],
  );
  if (claudeInstances.length === 0 && codexInstances.length === 0) return null;

  return (
    <section className={styles.root}>
      <h3 className={styles.sectionTitle}>
        <Icon icon={Gauge} size="md" tone="accent" />
        Provider Quotas
      </h3>
      <div className={`${styles.quotaGrid} rf-stagger`}>
        {claudeInstances.map((p) => (
          <ClaudeCodeInstanceCard
            key={`claude:${p.name}`}
            providerName={p.name}
            displayName={p.display_name}
          />
        ))}
        {codexInstances.map((p) => (
          <OpenAICodexInstanceCard
            key={`codex:${p.name}`}
            providerName={p.name}
            displayName={p.display_name}
          />
        ))}
      </div>
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
  const cacheEfficiency =
    t && t.total_prompt_tokens > 0
      ? (t.total_cache_read_tokens / t.total_prompt_tokens) * 100
      : 0;
  const cacheHitShare =
    t && t.total_tokens > 0
      ? (t.total_cache_read_tokens / t.total_tokens) * 100
      : 0;

  const topConversations = data?.top_conversations ?? [];

  return (
    <div className={styles.root}>
      <ProviderQuotaSection />
      <OpenCodeProviderQuotaSections />
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
                title="Cache Efficiency"
                value={formatPercent(cacheEfficiency)}
                subtitle="of prompt tokens served from cache"
                tone="success"
              />
              <StatCard
                title="Cache Hit Share"
                value={formatPercent(cacheHitShare)}
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
