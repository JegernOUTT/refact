import React from "react";
import { Box, Flex, Text } from "@radix-ui/themes";
import { useGetStatsSummaryQuery } from "../../../services/refact/stats";
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

type Props = { dateRange: DateRange };

export const OverviewTab: React.FC<Props> = ({ dateRange }) => {
  const { data, isLoading, isError } = useGetStatsSummaryQuery(
    dateRangeToApiArgs(dateRange),
  );

  if (isLoading) return <Spinner spinning />;
  if (isError) return <ErrorCallout>Failed to load stats</ErrorCallout>;

  if (!data || data.totals.total_calls === 0) {
    return (
      <Text className={styles.emptyText}>
        No usage data yet. Start chatting to see stats!
      </Text>
    );
  }

  const t = data.totals;
  const avgPerConversation =
    t.total_conversations > 0
      ? Math.round(t.total_tokens / t.total_conversations)
      : 0;
  const avgPerMessage =
    t.total_messages_sent > 0
      ? Math.round(t.total_tokens / t.total_messages_sent)
      : 0;
  const completionPct =
    t.total_tokens > 0
      ? Math.round((t.total_completion_tokens / t.total_tokens) * 100)
      : 0;
  const successRate =
    t.total_calls > 0
      ? Math.round((t.successful_calls / t.total_calls) * 100)
      : 0;
  const cacheEfficiency =
    t.total_tokens > 0
      ? Math.round((t.total_cache_read_tokens / t.total_tokens) * 100)
      : 0;

  const topConversations = data.top_conversations;

  return (
    <Flex direction="column" gap="4">
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
                      {formatCostDisplay(c.total_cost_usd, c.total_cost_coins)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Box>
        </Box>
      )}
    </Flex>
  );
};
