import React, { useMemo } from "react";
import { Box, Flex, Text, Badge } from "@radix-ui/themes";
import { useGetStatsSummaryQuery } from "../../../services/refact/stats";
import {
  useGetClaudeCodeUsageQuery,
  useGetOpenAICodexUsageQuery,
} from "../../../services/refact/providers";
import { useGetConfiguredProvidersQuery } from "../../../hooks";
import { Spinner } from "../../../components/Spinner";
import { ErrorCallout } from "../../../components/Callout";
import { StatCard } from "../components/StatCard";
import {
  formatTokenCount,
  formatCostDisplay,
  formatDuration,
} from "../utils/formatters";
import { dateRangeToApiArgs } from "../utils/dateRange";
import type { DateRange } from "../types";
import styles from "./OverviewTab.module.css";

const formatResetAt = (resetAt: string | null | undefined): string | null => {
  if (!resetAt) return null;
  const d = new Date(resetAt);
  if (isNaN(d.getTime())) return null;
  return `Resets ${d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  })}`;
};

const UsageBar: React.FC<{ pct: number }> = ({ pct }) => {
  const clamped = Math.max(0, Math.min(pct, 100));
  const color =
    clamped >= 90
      ? "var(--rf-color-danger)"
      : clamped >= 70
        ? "var(--rf-color-warning)"
        : "var(--rf-color-success)";
  return (
    <div
      style={{
        height: "4px",
        width: "100%",
        borderRadius: "2px",
        background: "var(--rf-surface-3)",
        overflow: "hidden",
        marginTop: "4px",
      }}
    >
      <div
        style={{
          height: "100%",
          width: `${clamped}%`,
          borderRadius: "2px",
          background: color,
          transition: "width 0.3s ease",
        }}
      />
    </div>
  );
};
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
    <Box
      style={{
        flex: "1 1 200px",
        minWidth: 0,
        background: "var(--rf-surface-1)",
        borderRadius: "var(--rf-radius-card)",
        padding: "var(--rf-space-3)",
      }}
    >
      <Flex align="center" gap="2" mb="2">
        <Text size="2" weight="medium">
          {displayName}
        </Text>
        <Text size="1" color="gray">
          ({providerName})
        </Text>
      </Flex>
      {data.five_hour &&
        (() => {
          const pct = Math.max(0, Math.min(data.five_hour.percent_used, 100));
          return (
            <Box mb="3">
              <Flex justify="between">
                <Text size="1" color="gray">
                  Session (5h)
                </Text>
                <Text size="1" color="gray">
                  {Math.round(pct)}%
                  {formatResetAt(data.five_hour.resets_at)
                    ? ` · ${formatResetAt(data.five_hour.resets_at)}`
                    : ""}
                </Text>
              </Flex>
              <UsageBar pct={pct} />
            </Box>
          );
        })()}
      {data.seven_day &&
        (() => {
          const pct = Math.max(0, Math.min(data.seven_day.percent_used, 100));
          return (
            <Box mb="2">
              <Flex justify="between">
                <Text size="1" color="gray">
                  Weekly
                </Text>
                <Text size="1" color="gray">
                  {Math.round(pct)}%
                  {formatResetAt(data.seven_day.resets_at)
                    ? ` · ${formatResetAt(data.seven_day.resets_at)}`
                    : ""}
                </Text>
              </Flex>
              <UsageBar pct={pct} />
            </Box>
          );
        })()}
      {data.extra_usage && (
        <Text size="1" color="gray">
          Extra: {data.extra_usage.is_enabled ? "on" : "off"} · $
          {data.extra_usage.used_credits.toFixed(2)} spent
          {typeof data.extra_usage.monthly_limit === "number"
            ? ` / $${data.extra_usage.monthly_limit.toFixed(0)}`
            : ""}
        </Text>
      )}
    </Box>
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

  return (
    <Box
      style={{
        flex: "1 1 200px",
        minWidth: 0,
        background: "var(--rf-surface-1)",
        borderRadius: "var(--rf-radius-card)",
        padding: "var(--rf-space-3)",
      }}
    >
      <Flex align="center" gap="2" mb="2">
        <Text size="2" weight="medium">
          {displayName}
        </Text>
        <Text size="1" color="gray">
          ({providerName})
        </Text>
        {data.plan_type && (
          <Badge color="blue" size="1">
            {data.plan_type}
          </Badge>
        )}
      </Flex>
      {data.rate_limit.primary_window &&
        (() => {
          const pct = Math.max(
            0,
            Math.min(data.rate_limit.primary_window.used_percent, 100),
          );
          return (
            <Box mb="3">
              <Flex justify="between" align="center">
                <Flex align="center" gap="1">
                  <Text size="1" color="gray">
                    Session (5h)
                  </Text>
                  {data.rate_limit.limit_reached && (
                    <Badge color="red" size="1">
                      Limit reached
                    </Badge>
                  )}
                </Flex>
                <Text size="1" color="gray">
                  {Math.round(pct)}%
                  {formatResetAt(data.rate_limit.primary_window.reset_at)
                    ? ` · ${formatResetAt(
                        data.rate_limit.primary_window.reset_at,
                      )}`
                    : ""}
                </Text>
              </Flex>
              <UsageBar pct={pct} />
            </Box>
          );
        })()}
      {data.rate_limit.secondary_window &&
        (() => {
          const pct = Math.max(
            0,
            Math.min(data.rate_limit.secondary_window.used_percent, 100),
          );
          return (
            <Box mb="2">
              <Flex justify="between">
                <Text size="1" color="gray">
                  Weekly
                </Text>
                <Text size="1" color="gray">
                  {Math.round(pct)}%
                  {formatResetAt(data.rate_limit.secondary_window.reset_at)
                    ? ` · ${formatResetAt(
                        data.rate_limit.secondary_window.reset_at,
                      )}`
                    : ""}
                </Text>
              </Flex>
              <UsageBar pct={pct} />
            </Box>
          );
        })()}
      {data.code_review_rate_limit?.primary_window &&
        (() => {
          const pct = Math.max(
            0,
            Math.min(
              data.code_review_rate_limit.primary_window.used_percent,
              100,
            ),
          );
          return (
            <Box mb="2">
              <Flex justify="between" align="center">
                <Flex align="center" gap="1">
                  <Text size="1" color="gray">
                    Code review
                  </Text>
                  {data.code_review_rate_limit.limit_reached && (
                    <Badge color="red" size="1">
                      Limit reached
                    </Badge>
                  )}
                </Flex>
                <Text size="1" color="gray">
                  {Math.round(pct)}%
                </Text>
              </Flex>
              <UsageBar pct={pct} />
            </Box>
          );
        })()}
      {data.credits && (
        <Text size="1" color="gray">
          Credits:{" "}
          {data.credits.unlimited
            ? "unlimited"
            : data.credits.has_credits
              ? `${data.credits.balance} remaining`
              : "none"}
        </Text>
      )}
    </Box>
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
    <Box>
      <Text
        size="3"
        weight="medium"
        className={styles.sectionTitle}
        mb="3"
        as="p"
      >
        Provider Quotas
      </Text>
      <Flex gap="3" wrap="wrap">
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
      </Flex>
    </Box>
  );
};

type Props = { dateRange: DateRange };

export const OverviewTab: React.FC<Props> = ({ dateRange }) => {
  const { data, isLoading, isError } = useGetStatsSummaryQuery(
    dateRangeToApiArgs(dateRange),
  );

  if (isLoading) return <Spinner spinning />;
  if (isError) return <ErrorCallout>Failed to load stats</ErrorCallout>;

  const t = data?.totals;
  const hasStats = !!(t && t.total_calls > 0);

  const avgPerConversation =
    t && t.total_conversations > 0
      ? Math.round(t.total_tokens / t.total_conversations)
      : 0;
  const avgPerMessage =
    t && t.total_messages_sent > 0
      ? Math.round(t.total_tokens / t.total_messages_sent)
      : 0;
  const completionPct =
    t && t.total_tokens > 0
      ? Math.round((t.total_completion_tokens / t.total_tokens) * 100)
      : 0;
  const successRate =
    t && t.total_calls > 0
      ? Math.round((t.successful_calls / t.total_calls) * 100)
      : 0;
  const cacheEfficiency =
    t && t.total_tokens > 0
      ? Math.round((t.total_cache_read_tokens / t.total_tokens) * 100)
      : 0;

  const topConversations = data?.top_conversations ?? [];

  return (
    <Flex direction="column" gap="4">
      <ProviderQuotaSection />
      {!hasStats && (
        <Text className={styles.emptyText}>
          No usage data yet. Start chatting to see stats!
        </Text>
      )}
      {hasStats && (
        <>
          <Flex className={styles.cardsRow}>
            <StatCard
              title="Total Usage"
              value={formatTokenCount(t.total_tokens)}
              subtitle={`${formatTokenCount(
                t.total_prompt_tokens,
              )} read + ${formatTokenCount(t.total_completion_tokens)} written`}
            />
            <StatCard
              title="Conversations"
              value={t.total_conversations.toString()}
              subtitle={`Each one used ~${formatTokenCount(
                avgPerConversation,
              )} tokens on average`}
            />
            <StatCard
              title="Messages Sent"
              value={t.total_messages_sent.toString()}
              subtitle={`Each message cost ~${formatTokenCount(
                avgPerMessage,
              )} tokens on average`}
            />
            <StatCard
              title="AI Wrote"
              value={formatTokenCount(t.total_completion_tokens)}
              subtitle={`${completionPct}% of total — most usage is from reading context`}
            />
            <StatCard
              title="Success Rate"
              value={`${successRate}%`}
              subtitle={`${t.successful_calls} of ${t.total_calls} calls succeeded`}
            />
            <StatCard
              title="Total Cost"
              value={formatCostDisplay(t.total_cost_usd)}
              subtitle="across all providers"
            />
            <StatCard
              title="Avg Duration"
              value={formatDuration(t.avg_duration_ms)}
              subtitle="average per LLM call"
            />
            <StatCard
              title="Cache Efficiency"
              value={`${cacheEfficiency}%`}
              subtitle={`${formatTokenCount(
                t.total_cache_read_tokens,
              )} tokens read from cache`}
            />
            <StatCard
              title="Cache Created"
              value={formatTokenCount(t.total_cache_creation_tokens)}
              subtitle="tokens written to cache for future reuse"
            />
          </Flex>

          {topConversations.length > 0 && (
            <Box>
              <Text
                size="3"
                weight="medium"
                className={styles.sectionTitle}
                mb="2"
                as="p"
              >
                Top Conversations by Token Usage
              </Text>
              <Box className={styles.tableWrapper}>
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
              </Box>
            </Box>
          )}
        </>
      )}
    </Flex>
  );
};
