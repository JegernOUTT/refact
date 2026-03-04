import React, { useMemo } from "react";
import { Badge, Flex, HoverCard, Skeleton, Text } from "@radix-ui/themes";
import { useGetStatsSummaryQuery } from "../../../../services/refact/stats";
import { useGetConfiguredProvidersQuery } from "../../../../hooks";
import { integrationsApi } from "../../../../services/refact/integrations";
import { useGetKnowledgeGraphQuery } from "../../../../services/refact/knowledgeGraphApi";
import { SparklineChart } from "./SparklineChart";
import { TokenDonut } from "./TokenDonut";
import { ModelBars } from "./ModelBars";
import { formatTokenCount } from "../../../StatsDashboard/utils/formatters";
import type { DashboardBreakpoint } from "../../types";
import type { ConversationStats } from "../../../StatsDashboard/types";
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

function formatRate(perDay: number): string {
  if (perDay < 0.01) return "<$0.01/day";
  return `~$${perDay.toFixed(2)}/day`;
}

function HoverStat({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <HoverCard.Root openDelay={300} closeDelay={100}>
      <HoverCard.Trigger>
        <span className={styles.hoverTrigger}>{label}</span>
      </HoverCard.Trigger>
      <HoverCard.Content size="1" side="top" align="center" className={styles.hoverContent} avoidCollisions>
        {children}
      </HoverCard.Content>
    </HoverCard.Root>
  );
}

export const StatsStrip: React.FC<StatsStripProps> = ({
  breakpoint,
  compact,
}) => {
  const todayKey = new Date().toDateString();
  // eslint-disable-next-line react-hooks/exhaustive-deps -- recalculate when day changes
  const from = useMemo(() => get7DaysAgo(), [todayKey]);
  const { data, isLoading, isError } = useGetStatsSummaryQuery({ from });
  const { data: providersData } = useGetConfiguredProvidersQuery();
  const { data: integrationsData } = integrationsApi.useGetAllIntegrationsQuery(undefined);
  const { data: knowledgeData } = useGetKnowledgeGraphQuery(undefined);

  const providerCount = providersData?.providers.length ?? 0;
  const integrationCount = integrationsData?.integrations.length ?? 0;
  const memoryCount = knowledgeData?.stats.active_docs ?? 0;

  const totalModels = useMemo(() => {
    if (!providersData?.providers) return 0;
    return providersData.providers.reduce((sum, p) => {
      return sum + p.model_count;
    }, 0);
  }, [providersData]);

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
          <>
            <div className={styles.card}>
              <Skeleton width="100%" height="100px" />
            </div>
            <div className={styles.card}>
              <Skeleton width="100%" height="100px" />
            </div>
          </>
        )}
      </div>
    );
  }

  const { totals, by_day, by_model, by_mode, top_conversations } = data;
  const successRate = totals.total_calls > 0
    ? Math.round((totals.successful_calls / totals.total_calls) * 100)
    : 0;
  const successColor = successRate >= 95 ? "green" : successRate >= 80 ? "amber" : "red";
  const costStr = formatCost(totals.total_cost_usd, totals.total_cost_coins);
  const failedCalls = totals.failed_calls;
  const cacheHitRate = totals.total_tokens > 0
    ? Math.round(((totals.total_cache_read_tokens) / totals.total_tokens) * 100)
    : 0;

  const dailyCostUsd = totals.total_cost_usd != null ? totals.total_cost_usd / 7 : 0;
  const dailyCostCoins = totals.total_cost_coins != null ? totals.total_cost_coins / 7 : 0;
  const hasUsageTracking = totals.total_calls > 0;

  if (compact) {
    return (
      <div className={styles.compactRow}>
        <Text size="1" color="gray">
          {totals.total_conversations} chats · {formatTokenCount(totals.total_tokens)} tok · {costStr}
          {totals.total_calls > 0 ? ` · ${successRate}% ok` : ""}
        </Text>
      </div>
    );
  }

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

  const topModes = [...by_mode].sort((a, b) => b.total_calls - a.total_calls).slice(0, 3);
  const totalModeCalls = topModes.reduce((s, m) => s + m.total_calls, 0) || 1;

  return (
    <div className={styles.statsGrid} data-breakpoint={breakpoint}>
      {/* Card 1: 7-Day Activity */}
      <div className={styles.card}>
        <Text size="1" weight="bold" color="gray" className={styles.cardTitle}>7-DAY ACTIVITY</Text>

        <div className={styles.cardSection}>
          <Flex justify="between" align="center">
            <HoverStat label={`${totals.total_conversations} conversations`}>
              <Flex direction="column" gap="1">
                <Text size="2" weight="bold">Conversations</Text>
                <Text size="1" color="gray">Total unique chat sessions in the last 7 days.</Text>
                <Text size="1">{totals.total_messages_sent} user messages sent across all chats.</Text>
              </Flex>
            </HoverStat>
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
          <HoverStat label={`${formatTokenCount(totals.total_tokens)} total tokens`}>
            <Flex direction="column" gap="1">
              <Text size="2" weight="bold">Token Breakdown</Text>
              <Text size="1">Prompt: {formatTokenCount(totals.total_prompt_tokens)}</Text>
              <Text size="1">Completion: {formatTokenCount(totals.total_completion_tokens)}</Text>
              <Text size="1">Cache read: {formatTokenCount(totals.total_cache_read_tokens)}</Text>
              <Text size="1">Cache created: {formatTokenCount(totals.total_cache_creation_tokens)}</Text>
              {cacheHitRate > 0 && (
                <Text size="1" color="gray">Cache hit rate: {cacheHitRate}% of tokens served from cache.</Text>
              )}
              {!hasUsageTracking && (
                <Text size="1" color="amber">Note: Not all threads have tracked usage data.</Text>
              )}
            </Flex>
          </HoverStat>
        </div>

        <div className={styles.cardDivider} />

        <div className={styles.cardSection}>
          {totals.total_calls > 0 && (
            <Flex justify="between" align="center">
              <HoverStat label={`${successRate}% success`}>
                <Flex direction="column" gap="1">
                  <Text size="2" weight="bold">LLM Call Success Rate</Text>
                  <Text size="1" color="gray">
                    Percentage of successful LLM API calls out of all attempts.
                    Failures include network errors, rate limits, and model errors.
                  </Text>
                  <Text size="1">{totals.successful_calls} succeeded / {totals.total_calls} total calls</Text>
                  {failedCalls > 0 && (
                    <Text size="1" color="red">{failedCalls} failed calls (retries, timeouts, rate limits)</Text>
                  )}
                </Flex>
              </HoverStat>
              <Badge size="1" color={successColor} variant="soft">{successRate}%</Badge>
            </Flex>
          )}
          {totals.avg_duration_ms > 0 && (
            <HoverStat label={`Avg ${Math.round(totals.avg_duration_ms)}ms response`}>
              <Flex direction="column" gap="1">
                <Text size="2" weight="bold">Average Response Time</Text>
                <Text size="1" color="gray">
                  Mean duration of LLM API calls, from request to full response.
                  Includes network latency and model inference time.
                </Text>
              </Flex>
            </HoverStat>
          )}
          <SparklineChart days={by_day} />
        </div>
      </div>

      {/* Card 2: Project Pulse */}
      <div className={styles.card}>
        <Text size="1" weight="bold" color="gray" className={styles.cardTitle}>PROJECT PULSE</Text>

        {topModes.length > 0 && (
          <div className={styles.cardSection}>
            <HoverStat label={`${by_mode.length} modes used`}>
              <Flex direction="column" gap="1">
                <Text size="2" weight="bold">Agent Modes</Text>
                <Text size="1" color="gray">
                  Different modes determine which tools and prompts the AI uses.
                  Common modes: Agent (full tools), Explore (read-only), Chat (no tools).
                </Text>
                {by_mode.map((m) => (
                  <Flex key={m.mode} justify="between" gap="2">
                    <Text size="1">{m.mode}</Text>
                    <Text size="1" color="gray">{m.total_calls} calls</Text>
                  </Flex>
                ))}
              </Flex>
            </HoverStat>
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
            <HoverStat label={`${providerCount} providers available`}>
              <Flex direction="column" gap="1">
                <Text size="2" weight="bold">LLM Providers</Text>
                <Text size="1" color="gray">
                  Configured LLM providers (e.g. OpenAI, Anthropic, local models).
                </Text>
                {totalModels > 0 && (
                  <Text size="1">{totalModels} models available across all providers.</Text>
                )}
              </Flex>
            </HoverStat>
          )}
          {integrationCount > 0 && (
            <HoverStat label={`${integrationCount} integrations`}>
              <Flex direction="column" gap="1">
                <Text size="2" weight="bold">Integrations</Text>
                <Text size="1" color="gray">
                  Connected tools and services: GitHub, Docker, databases, MCP servers, etc.
                </Text>
              </Flex>
            </HoverStat>
          )}
          {memoryCount > 0 && (
            <HoverStat label={`${memoryCount} memories`}>
              <Flex direction="column" gap="1">
                <Text size="2" weight="bold">Knowledge Memories</Text>
                <Text size="1" color="gray">
                  Persistent knowledge entries the AI remembers across sessions.
                  Includes project patterns, decisions, and learned preferences.
                </Text>
              </Flex>
            </HoverStat>
          )}
        </div>
      </div>

      {/* Card 3: Spending */}
      <div className={styles.card}>
        <Text size="1" weight="bold" color="gray" className={styles.cardTitle}>SPENDING</Text>

        <div className={styles.cardSection}>
          <HoverStat label={`Total: ${costStr}`}>
            <Flex direction="column" gap="1">
              <Text size="2" weight="bold">7-Day Cost</Text>
              {totals.total_cost_usd != null && totals.total_cost_usd > 0 && (
                <Text size="1">USD: ${totals.total_cost_usd.toFixed(4)}</Text>
              )}
              {totals.total_cost_coins != null && totals.total_cost_coins > 0 && (
                <Text size="1">Coins: {totals.total_cost_coins.toFixed(1)}</Text>
              )}
              <Text size="1" color="gray">
                Cost is calculated per LLM API call based on token usage.
                Not all conversations may have tracked cost data.
              </Text>
            </Flex>
          </HoverStat>
          <HoverStat label={`Rate: ${dailyCostUsd > 0 ? formatRate(dailyCostUsd) : dailyCostCoins > 0 ? `~${Math.round(dailyCostCoins)} coins/day` : "free"}`}>
            <Flex direction="column" gap="1">
              <Text size="2" weight="bold">Daily Spend Rate</Text>
              <Text size="1" color="gray">
                Average daily cost over the last 7 days.
                Actual daily spend varies based on usage patterns.
              </Text>
            </Flex>
          </HoverStat>
        </div>

        <div className={styles.cardDivider} />

        {top_conversations.length > 0 && (
          <div className={styles.cardSection}>
            <Text size="1" color="gray">Top spenders:</Text>
            {top_conversations.slice(0, 3).map((conv: ConversationStats) => {
              const convCost = formatCost(conv.total_cost_usd, conv.total_cost_coins);
              const shortModel = conv.model_id.split("/").pop() ?? conv.model_id;
              return (
                <Flex key={conv.chat_id} justify="between" align="center" gap="1">
                  <Text size="1" truncate style={{ flex: 1, minWidth: 0 }}>
                    {shortModel}
                  </Text>
                  <Text size="1" color="gray" style={{ flexShrink: 0 }}>
                    {formatTokenCount(conv.total_tokens)} tok · {convCost}
                  </Text>
                </Flex>
              );
            })}
          </div>
        )}

        <div className={styles.cardDivider} />

        <div className={styles.cardSection}>
          <HoverStat label={`${formatTokenCount(totals.total_prompt_tokens)} prompt tokens`}>
            <Flex direction="column" gap="1">
              <Text size="2" weight="bold">Prompt Tokens</Text>
              <Text size="1" color="gray">
                Tokens sent to the LLM (system prompt + conversation context + tool results).
                This is typically the largest cost component.
              </Text>
            </Flex>
          </HoverStat>
          <HoverStat label={`${formatTokenCount(totals.total_completion_tokens)} completion tokens`}>
            <Flex direction="column" gap="1">
              <Text size="2" weight="bold">Completion Tokens</Text>
              <Text size="1" color="gray">
                Tokens generated by the LLM (responses, tool calls, reasoning).
                Usually 3-5x more expensive per token than prompt tokens.
              </Text>
            </Flex>
          </HoverStat>
          {cacheHitRate > 0 && (
            <HoverStat label={`${cacheHitRate}% cache hit rate`}>
              <Flex direction="column" gap="1">
                <Text size="2" weight="bold">Cache Efficiency</Text>
                <Text size="1" color="gray">
                  Percentage of tokens served from provider cache (Anthropic prompt caching, etc.).
                  Cached tokens are significantly cheaper than fresh computation.
                </Text>
                <Text size="1">{formatTokenCount(totals.total_cache_read_tokens)} tokens read from cache.</Text>
              </Flex>
            </HoverStat>
          )}
        </div>
      </div>
    </div>
  );
};
