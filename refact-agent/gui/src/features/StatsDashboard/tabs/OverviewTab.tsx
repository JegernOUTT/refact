import React from "react";
import { Flex, Text } from "@radix-ui/themes";
import { useGetStatsSummaryQuery } from "../../../services/refact/stats";
import { Spinner } from "../../../components/Spinner";
import { ErrorCallout } from "../../../components/Callout";
import { StatCard } from "../components/StatCard";
import { formatTokenCount } from "../utils/formatters";
import type { DateRange } from "../types";
import styles from "./OverviewTab.module.css";

type Props = { dateRange: DateRange };

function dateRangeArgs(dateRange: DateRange): { from?: string; to?: string } {
  if (dateRange.preset === "all") return {};
  const days = dateRange.preset === "7d" ? 7 : 30;
  const from = new Date(Date.now() - days * 24 * 60 * 60 * 1000)
    .toISOString()
    .slice(0, 10);
  return { from };
}

export const OverviewTab: React.FC<Props> = ({ dateRange }) => {
  const { data, isLoading, isError } = useGetStatsSummaryQuery(
    dateRangeArgs(dateRange),
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

  return (
    <Flex direction="column" gap="4">
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
      </Flex>
    </Flex>
  );
};
