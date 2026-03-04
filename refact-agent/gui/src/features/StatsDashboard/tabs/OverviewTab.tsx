import React from "react";
import { Box, Flex, Text, Badge } from "@radix-ui/themes";
import { useGetStatsSummaryQuery } from "../../../services/refact/stats";
import {
  useGetClaudeCodeUsageQuery,
  useGetOpenAICodexUsageQuery,
} from "../../../services/refact/providers";
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
  return `Resets ${d.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}`;
};

const UsageBar: React.FC<{ pct: number }> = ({ pct }) => {
  const clamped = Math.max(0, Math.min(pct, 100));
  const color = clamped >= 90 ? "var(--red-9)" : clamped >= 70 ? "var(--orange-9)" : "var(--green-9)";
  return (
    <div style={{ height: "4px", width: "100%", borderRadius: "2px", background: "var(--gray-a4)", overflow: "hidden", marginTop: "4px" }}>
      <div style={{ height: "100%", width: `${clamped}%`, borderRadius: "2px", background: color, transition: "width 0.3s ease" }} />
    </div>
  );
};

const ProviderQuotaSection: React.FC = () => {
  const { data: claudeUsage } = useGetClaudeCodeUsageQuery(undefined, { pollingInterval: 5 * 60_000 });
  const { data: codexUsage } = useGetOpenAICodexUsageQuery(undefined, { pollingInterval: 5 * 60_000 });

  const hasClaudeData = !!(claudeUsage?.data && (claudeUsage.data.five_hour || claudeUsage.data.seven_day));
  const hasCodexData = !!(codexUsage?.data?.rate_limit);

  if (!hasClaudeData && !hasCodexData) return null;

  return (
    <Box>
      <Text size="3" weight="medium" className={styles.sectionTitle} mb="3" as="p">
        Provider Quotas
      </Text>
      <Flex gap="3" wrap="wrap">
        {hasClaudeData && claudeUsage?.data && (
          <Box style={{ flex: "1 1 200px", minWidth: "180px", background: "var(--gray-a2)", borderRadius: "var(--radius-3)", padding: "var(--space-3)" }}>
            <Text size="2" weight="medium" as="p" mb="2">Claude Code</Text>
            {claudeUsage.data.five_hour && (() => {
              const pct = Math.max(0, Math.min(claudeUsage.data!.five_hour!.percent_used, 100));
              return (
                <Box mb="3">
                  <Flex justify="between">
                    <Text size="1" color="gray">Session (5h)</Text>
                    <Text size="1" color="gray">{Math.round(pct)}%{formatResetAt(claudeUsage.data!.five_hour!.resets_at) ? ` · ${formatResetAt(claudeUsage.data!.five_hour!.resets_at)}` : ""}</Text>
                  </Flex>
                  <UsageBar pct={pct} />
                </Box>
              );
            })()}
            {claudeUsage.data.seven_day && (() => {
              const pct = Math.max(0, Math.min(claudeUsage.data!.seven_day!.percent_used, 100));
              return (
                <Box mb="2">
                  <Flex justify="between">
                    <Text size="1" color="gray">Weekly</Text>
                    <Text size="1" color="gray">{Math.round(pct)}%{formatResetAt(claudeUsage.data!.seven_day!.resets_at) ? ` · ${formatResetAt(claudeUsage.data!.seven_day!.resets_at)}` : ""}</Text>
                  </Flex>
                  <UsageBar pct={pct} />
                </Box>
              );
            })()}
            {claudeUsage.data.extra_usage && (
              <Text size="1" color="gray">
                Extra: {claudeUsage.data.extra_usage.is_enabled ? "on" : "off"} · ${claudeUsage.data.extra_usage.used_credits.toFixed(2)} spent
                {typeof claudeUsage.data.extra_usage.monthly_limit === "number"
                  ? ` / $${claudeUsage.data.extra_usage.monthly_limit.toFixed(0)}`
                  : ""}
              </Text>
            )}
          </Box>
        )}

        {hasCodexData && codexUsage?.data && (
          <Box style={{ flex: "1 1 200px", minWidth: "180px", background: "var(--gray-a2)", borderRadius: "var(--radius-3)", padding: "var(--space-3)" }}>
            <Flex align="center" gap="2" mb="2">
              <Text size="2" weight="medium">OpenAI Codex</Text>
              {codexUsage.data.plan_type && <Badge color="blue" size="1">{codexUsage.data.plan_type}</Badge>}
            </Flex>
            {codexUsage.data.rate_limit?.primary_window && (() => {
              const pct = Math.max(0, Math.min(codexUsage.data!.rate_limit!.primary_window!.used_percent, 100));
              return (
                <Box mb="3">
                  <Flex justify="between" align="center">
                    <Flex align="center" gap="1">
                      <Text size="1" color="gray">Session (5h)</Text>
                      {codexUsage.data!.rate_limit!.limit_reached && <Badge color="red" size="1">Limit reached</Badge>}
                    </Flex>
                    <Text size="1" color="gray">{Math.round(pct)}%{formatResetAt(codexUsage.data!.rate_limit!.primary_window!.reset_at) ? ` · ${formatResetAt(codexUsage.data!.rate_limit!.primary_window!.reset_at)}` : ""}</Text>
                  </Flex>
                  <UsageBar pct={pct} />
                </Box>
              );
            })()}
            {codexUsage.data.rate_limit?.secondary_window && (() => {
              const pct = Math.max(0, Math.min(codexUsage.data!.rate_limit!.secondary_window!.used_percent, 100));
              return (
                <Box mb="2">
                  <Flex justify="between">
                    <Text size="1" color="gray">Weekly</Text>
                    <Text size="1" color="gray">{Math.round(pct)}%{formatResetAt(codexUsage.data!.rate_limit!.secondary_window!.reset_at) ? ` · ${formatResetAt(codexUsage.data!.rate_limit!.secondary_window!.reset_at)}` : ""}</Text>
                  </Flex>
                  <UsageBar pct={pct} />
                </Box>
              );
            })()}
            {codexUsage.data.code_review_rate_limit?.primary_window && (() => {
              const pct = Math.max(0, Math.min(codexUsage.data!.code_review_rate_limit!.primary_window!.used_percent, 100));
              return (
                <Box mb="2">
                  <Flex justify="between" align="center">
                    <Flex align="center" gap="1">
                      <Text size="1" color="gray">Code review</Text>
                      {codexUsage.data!.code_review_rate_limit!.limit_reached && <Badge color="red" size="1">Limit reached</Badge>}
                    </Flex>
                    <Text size="1" color="gray">{Math.round(pct)}%</Text>
                  </Flex>
                  <UsageBar pct={pct} />
                </Box>
              );
            })()}
            {codexUsage.data.credits && (
              <Text size="1" color="gray">
                Credits: {codexUsage.data.credits.unlimited ? "unlimited" : codexUsage.data.credits.has_credits ? `${codexUsage.data.credits.balance} remaining` : "none"}
              </Text>
            )}
          </Box>
        )}
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

  const avgPerConversation = t && t.total_conversations > 0
    ? Math.round(t.total_tokens / t.total_conversations)
    : 0;
  const avgPerMessage = t && t.total_messages_sent > 0
    ? Math.round(t.total_tokens / t.total_messages_sent)
    : 0;
  const completionPct = t && t.total_tokens > 0
    ? Math.round((t.total_completion_tokens / t.total_tokens) * 100)
    : 0;
  const successRate = t && t.total_calls > 0
    ? Math.round((t.successful_calls / t.total_calls) * 100)
    : 0;
  const cacheEfficiency = t && t.total_tokens > 0
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
      {hasStats && t && (
        <>
          <Flex className={styles.cardsRow}>
            <StatCard
              title="Total Usage"
              value={formatTokenCount(t.total_tokens)}
              subtitle={`${formatTokenCount(t.total_prompt_tokens)} read + ${formatTokenCount(t.total_completion_tokens)} written`}
            />
            <StatCard
              title="Conversations"
              value={t.total_conversations.toString()}
              subtitle={`Each one used ~${formatTokenCount(avgPerConversation)} tokens on average`}
            />
            <StatCard
              title="Messages Sent"
              value={t.total_messages_sent.toString()}
              subtitle={`Each message cost ~${formatTokenCount(avgPerMessage)} tokens on average`}
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
              value={formatCostDisplay(t.total_cost_usd, t.total_cost_coins)}
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
              subtitle={`${formatTokenCount(t.total_cache_read_tokens)} tokens read from cache`}
            />
            <StatCard
              title="Cache Created"
              value={formatTokenCount(t.total_cache_creation_tokens)}
              subtitle="tokens written to cache for future reuse"
            />
          </Flex>

          {topConversations.length > 0 && (
            <Box>
              <Text size="3" weight="medium" className={styles.sectionTitle} mb="2" as="p">
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
                        <td className={styles.td}>{formatTokenCount(c.total_tokens)}</td>
                        <td className={styles.td}>{formatCostDisplay(c.total_cost_usd, c.total_cost_coins)}</td>
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
