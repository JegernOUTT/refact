import React, { useMemo } from "react";
import { Badge, Flex, Skeleton, Text } from "@radix-ui/themes";
import { useGetStatsSummaryQuery } from "../../../../services/refact/stats";
import { useGetConfiguredProvidersQuery } from "../../../../hooks";
import { integrationsApi } from "../../../../services/refact/integrations";
import { useGetKnowledgeGraphQuery } from "../../../../services/refact/knowledgeGraphApi";
import { SparklineChart } from "./SparklineChart";
import { TokenDonut } from "./TokenDonut";
import { ModelBars } from "./ModelBars";
import { formatTokenCount } from "../../../StatsDashboard/utils/formatters";
import type { DashboardBreakpoint } from "../../types";
import styles from "./StatsStrip.module.css";

type StatsStripProps = {
  breakpoint: DashboardBreakpoint;
  compact?: boolean;
};

function get7DaysAgo(): string {
  const d = new Date();
  d.setDate(d.getDate() - 7);
  const yyyy = d.getFullYear();
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  return `${yyyy}-${mm}-${dd}`;
}

function formatCost(usd: number | null, coins: number | null): string {
  const parts: string[] = [];
  if (usd != null && usd > 0) parts.push(`$${usd.toFixed(2)}`);
  if (coins != null && coins > 0) parts.push(`${formatTokenCount(coins)} coins`);
  if (parts.length === 0) return "free";
  return parts.join(" / ");
}

export const StatsStrip: React.FC<StatsStripProps> = ({
  breakpoint,
  compact,
}) => {
  const from = useMemo(() => get7DaysAgo(), []);
  const { data, isLoading, isError } = useGetStatsSummaryQuery({ from });
  const { data: providersData } = useGetConfiguredProvidersQuery();
  const { data: integrationsData } = integrationsApi.useGetAllIntegrationsQuery(undefined);
  const { data: knowledgeData } = useGetKnowledgeGraphQuery(undefined);

  const providerCount = providersData?.providers.length ?? 0;
  const integrationCount = integrationsData?.integrations.length ?? 0;
  const memoryCount = knowledgeData?.stats.active_docs ?? 0;

  if (isError) {
    return (
      <div className={styles.compactRow}>
        <Text size="1" color="red">Failed to load stats</Text>
      </div>
    );
  }

  if (isLoading || !data) {
    if (compact) {
      return (
        <div className={styles.compactRow}>
          <Skeleton><Text size="1">Loading stats...</Text></Skeleton>
        </div>
      );
    }
    return (
      <div className={styles.statsGrid} data-breakpoint={breakpoint}>
        <div className={styles.card}>
          <Skeleton width="100%" height="100px" />
        </div>
        {breakpoint !== "narrow" && (
          <div className={styles.card}>
            <Skeleton width="100%" height="100px" />
          </div>
        )}
      </div>
    );
  }

  const { totals, by_day, by_model, by_mode } = data;
  const successRate = totals.total_calls > 0
    ? Math.round((totals.successful_calls / totals.total_calls) * 100)
    : 0;
  const successColor = successRate >= 95 ? "green" : successRate >= 80 ? "amber" : "red";
  const costStr = formatCost(totals.total_cost_usd, totals.total_cost_coins);

  // Compact: single line
  if (compact) {
    return (
      <div className={styles.compactRow}>
        <Text size="1" color="gray">
          {totals.total_conversations} chats · {formatTokenCount(totals.total_tokens)} tok · {costStr}
          {totals.total_calls > 0 ? ` · ${successRate}% success` : ""}
          {providerCount > 0 ? ` · ${providerCount} providers` : ""}
        </Text>
      </div>
    );
  }

  // Narrow: single stacked card
  if (breakpoint === "narrow") {
    return (
      <div className={styles.narrowStats}>
        <Flex justify="between" align="center">
          <Text size="1" color="gray">{totals.total_conversations} chats · {formatTokenCount(totals.total_tokens)} tok</Text>
          <Text size="1" color="gray">{costStr}</Text>
        </Flex>
        <Flex justify="between" align="center" gap="2">
          {totals.total_calls > 0 && (
            <Badge size="1" color={successColor} variant="soft">{successRate}% success</Badge>
          )}
          {providerCount > 0 && <Text size="1" color="gray">{providerCount} providers</Text>}
        </Flex>
        <SparklineChart days={by_day} />
      </div>
    );
  }

  // Medium/Wide: 2-column cards with internal dividers
  const topModes = [...by_mode].sort((a, b) => b.total_calls - a.total_calls).slice(0, 3);
  const totalModeCalls = topModes.reduce((s, m) => s + m.total_calls, 0) || 1;

  return (
    <div className={styles.statsGrid} data-breakpoint={breakpoint}>
      {/* Left: 7-Day Stats */}
      <div className={styles.card}>
        <Text size="1" weight="bold" color="gray" className={styles.cardTitle}>7-DAY STATS</Text>

        <div className={styles.cardSection}>
          <Flex justify="between" align="center">
            <Text size="2" weight="medium">{totals.total_conversations} conversations</Text>
            <Text size="1" color="gray">{totals.total_messages_sent} messages</Text>
          </Flex>
        </div>

        <div className={styles.cardDivider} />

        <div className={styles.cardSection}>
          <Flex align="center" gap="3">
            <TokenDonut
              prompt={totals.total_prompt_tokens}
              completion={totals.total_completion_tokens}
              cache={totals.total_cache_read_tokens + totals.total_cache_creation_tokens}
            />
          </Flex>
          <Flex justify="between" align="center">
            <Text size="1" color="gray">Cost: {costStr}</Text>
            {totals.total_calls > 0 && (
              <Badge size="1" color={successColor} variant="soft">{successRate}% success</Badge>
            )}
          </Flex>
        </div>

        <div className={styles.cardDivider} />

        <div className={styles.cardSection}>
          {totals.avg_duration_ms > 0 && (
            <Text size="1" color="gray">Avg response: {Math.round(totals.avg_duration_ms)}ms</Text>
          )}
          <SparklineChart days={by_day} />
        </div>
      </div>

      {/* Right: Project Pulse */}
      <div className={styles.card}>
        <Text size="1" weight="bold" color="gray" className={styles.cardTitle}>PROJECT PULSE</Text>

        {topModes.length > 0 && (
          <div className={styles.cardSection}>
            <Text size="1" color="gray">Modes used:</Text>
            {topModes.map((m) => {
              const pct = Math.round((m.total_calls / totalModeCalls) * 100);
              return (
                <Flex key={m.mode} align="center" gap="2">
                  <div
                    className={styles.modeFill}
                    style={{ width: `${Math.max(pct, 4)}%` }}
                  />
                  <Text size="1" truncate>{m.mode} ({pct}%)</Text>
                </Flex>
              );
            })}
          </div>
        )}

        <div className={styles.cardDivider} />

        <div className={styles.cardSection}>
          <ModelBars models={by_model} />
        </div>

        <div className={styles.cardDivider} />

        <div className={styles.cardSection}>
          {providerCount > 0 && (
            <Text size="1" color="gray">{providerCount} providers active</Text>
          )}
          {integrationCount > 0 && (
            <Text size="1" color="gray">{integrationCount} integrations configured</Text>
          )}
          {memoryCount > 0 && (
            <Text size="1" color="gray">{memoryCount} knowledge memories</Text>
          )}
        </div>
      </div>
    </div>
  );
};
