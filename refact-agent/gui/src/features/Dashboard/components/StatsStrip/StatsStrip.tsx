import React, { useMemo } from "react";
import { DashboardBadge as Badge, DashboardFlex as Flex, DashboardHoverCard as HoverCard, DashboardSkeleton as Skeleton, DashboardText as Text } from "../DashboardPrimitives";
import { useGetStatsSummaryQuery } from "../../../../services/refact/stats";
import { useGetConfiguredProvidersQuery } from "../../../../hooks";
import {
  useGetClaudeCodeUsageQuery,
  useGetOpenAICodexUsageQuery,
} from "../../../../services/refact/providers";
import { integrationsApi } from "../../../../services/refact/integrations";
import { useGetKnowledgeGraphQuery } from "../../../../services/refact/knowledgeGraphApi";
import { useGetCapsQuery } from "../../../../services/refact/caps";
import { useAppDispatch } from "../../../../hooks";
import { push } from "../../../Pages/pagesSlice";
import { SparklineChart } from "./SparklineChart";
import { TokenDonut } from "./TokenDonut";
import { ModelBars } from "./ModelBars";
import { MiniDonut } from "./MiniDonut";
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

function formatCost(usd: number | null): string {
  if (usd != null && usd > 0) return `$${usd.toFixed(2)}`;
  return "free";
}

function formatRate(perDay: number): string {
  if (perDay < 0.01) return "<$0.01/day";
  return `~$${perDay.toFixed(2)}/day`;
}

function HoverStat({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <HoverCard.Root openDelay={300} closeDelay={100}>
      <HoverCard.Trigger>
        <span className={styles.hoverTrigger}>{label}</span>
      </HoverCard.Trigger>
      <HoverCard.Content
        size="1"
        side="top"
        align="center"
        className={styles.hoverContent}
        avoidCollisions
      >
        {children}
      </HoverCard.Content>
    </HoverCard.Root>
  );
}

function formatResetAt(resetAt: string | null | undefined): string | null {
  if (!resetAt) return null;
  const d = new Date(resetAt);
  if (isNaN(d.getTime())) return null;
  return `Resets ${d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  })}`;
}

function UsageBar({ pct }: { pct: number }) {
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
        height: "3px",
        width: "100%",
        borderRadius: "2px",
        background: "var(--rf-surface-2)",
        overflow: "hidden",
        marginTop: "3px",
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
}

function WindowRow({
  label,
  pct,
  resetAt,
  limitReached,
}: {
  label: string;
  pct: number;
  resetAt?: string | null;
  limitReached?: boolean;
}) {
  const clamped = Math.max(0, Math.min(pct, 100));
  const reset = formatResetAt(resetAt);
  return (
    <div style={{ marginBottom: "6px" }}>
      <Flex justify="between" align="center">
        <Flex align="center" gap="1">
          <Text size="1" tone="muted">
            {label}
          </Text>
          {limitReached && (
            <Badge tone="danger" size="1">
              Limit
            </Badge>
          )}
        </Flex>
        <Text size="1" tone="muted">
          {Math.round(clamped)}%{reset ? ` · ${reset}` : ""}
        </Text>
      </Flex>
      <UsageBar pct={clamped} />
    </div>
  );
}

function ModelRow({
  label,
  model,
  explanation,
}: {
  label: string;
  model: string;
  explanation: string;
}) {
  const shortName = model.split("/").pop() ?? model;
  return (
    <HoverCard.Root openDelay={300} closeDelay={100}>
      <HoverCard.Trigger>
        <Flex align="center" gap="2" style={{ cursor: "help" }}>
          <Text size="1" tone="muted" style={{ minWidth: 70, flexShrink: 0 }}>
            {label}
          </Text>
          <Text size="1" weight="medium" truncate>
            {shortName}
          </Text>
        </Flex>
      </HoverCard.Trigger>
      <HoverCard.Content
        size="1"
        side="top"
        align="center"
        className={styles.hoverContent}
        avoidCollisions
      >
        <Flex direction="column" gap="1">
          <Text size="2" weight="bold">
            {label}
          </Text>
          <Text size="1" tone="muted">
            {explanation}
          </Text>
          <Text size="1">Current: {model}</Text>
        </Flex>
      </HoverCard.Content>
    </HoverCard.Root>
  );
}

function DefaultModelsCard() {
  const dispatch = useAppDispatch();
  const { data: caps, isLoading } = useGetCapsQuery(undefined);

  return (
    <div className={styles.card}>
      <Flex justify="between" align="center" className={styles.cardTitle}>
        <Text size="1" weight="bold" tone="muted">
          DEFAULT MODELS
        </Text>
        <button
          type="button"
          className={styles.configureButton}
          onClick={() => dispatch(push({ name: "default models" }))}
        >
          <Text size="1">Configure</Text>
        </button>
      </Flex>

      {isLoading || !caps ? (
        <Flex direction="column" gap="2">
          <Skeleton height="16px" />
          <Skeleton height="16px" />
        </Flex>
      ) : (
        <div className={styles.cardSection}>
          {caps.chat_default_model && (
            <ModelRow
              label="Chat"
              model={caps.chat_default_model}
              explanation="Primary model for chat conversations."
            />
          )}
          {caps.chat_model_2 &&
            caps.chat_model_2 !== caps.chat_default_model && (
              <ModelRow
                label="Chat 2"
                model={caps.chat_model_2}
                explanation="Secondary chat model slot for future chat workflows."
              />
            )}
          {caps.task_planner_agent_model &&
            caps.task_planner_agent_model !== caps.chat_default_model &&
            caps.task_planner_agent_model !== caps.chat_model_2 && (
              <ModelRow
                label="Task Agent"
                model={caps.task_planner_agent_model}
                explanation="Model used by task management when spawning task agents."
              />
            )}
          {caps.chat_thinking_model &&
            caps.chat_thinking_model !== caps.chat_default_model && (
              <ModelRow
                label="Thinking"
                model={caps.chat_thinking_model}
                explanation="Model with extended reasoning for complex tasks."
              />
            )}
          {caps.chat_light_model &&
            caps.chat_light_model !== caps.chat_default_model && (
              <ModelRow
                label="Light"
                model={caps.chat_light_model}
                explanation="Faster, cheaper model for simple tasks."
              />
            )}
          {caps.chat_buddy_model &&
            caps.chat_buddy_model !== caps.chat_default_model &&
            caps.chat_buddy_model !== caps.chat_light_model && (
              <ModelRow
                label="Companion"
                model={caps.chat_buddy_model}
                explanation="Model used by your companion for background tasks."
              />
            )}
          {caps.completion_default_model && (
            <ModelRow
              label="Completion"
              model={caps.completion_default_model}
              explanation="Model for inline code completion."
            />
          )}
          <Text size="1" tone="muted">
            {Object.keys(caps.chat_models).length} chat +{" "}
            {Object.keys(caps.completion_models).length} completion available
          </Text>
        </div>
      )}
    </div>
  );
}

function ClaudeCodeInstanceRow({
  providerName,
  displayName,
}: {
  providerName: string;
  displayName: string;
}) {
  const { data: claudeUsage } = useGetClaudeCodeUsageQuery(
    { providerName },
    { pollingInterval: 5 * 60_000 },
  );
  const data = claudeUsage?.data;
  if (!data) return null;
  if (!data.five_hour && !data.seven_day) return null;

  return (
    <div className={styles.cardSection}>
      <Flex align="center" gap="2" mb="1">
        <Text size="1" weight="medium">
          {displayName}
        </Text>
        <Text size="1" tone="muted">
          ({providerName})
        </Text>
      </Flex>
      {data.five_hour && (
        <WindowRow
          label="Session (5h)"
          pct={data.five_hour.percent_used}
          resetAt={data.five_hour.resets_at}
        />
      )}
      {data.seven_day && (
        <WindowRow
          label="Weekly"
          pct={data.seven_day.percent_used}
          resetAt={data.seven_day.resets_at}
        />
      )}
      {data.extra_usage && (
        <Text size="1" tone="muted">
          Extra: {data.extra_usage.is_enabled ? "on" : "off"} · $
          {data.extra_usage.used_credits.toFixed(2)} spent
          {typeof data.extra_usage.monthly_limit === "number"
            ? ` / $${data.extra_usage.monthly_limit.toFixed(0)}`
            : ""}
        </Text>
      )}
    </div>
  );
}

function OpenAICodexInstanceRow({
  providerName,
  displayName,
}: {
  providerName: string;
  displayName: string;
}) {
  const { data: codexUsage } = useGetOpenAICodexUsageQuery(
    { providerName },
    { pollingInterval: 5 * 60_000 },
  );
  const data = codexUsage?.data;
  if (!data?.rate_limit) return null;

  return (
    <div className={styles.cardSection}>
      <Flex align="center" gap="2" mb="1">
        <Text size="1" weight="medium">
          {displayName}
        </Text>
        <Text size="1" tone="muted">
          ({providerName})
        </Text>
        {data.plan_type && (
          <Badge color="blue" size="1">
            {data.plan_type}
          </Badge>
        )}
      </Flex>
      {data.rate_limit.primary_window && (
        <WindowRow
          label="Session (5h)"
          pct={data.rate_limit.primary_window.used_percent}
          resetAt={data.rate_limit.primary_window.reset_at}
          limitReached={data.rate_limit.limit_reached}
        />
      )}
      {data.rate_limit.secondary_window && (
        <WindowRow
          label="Weekly"
          pct={data.rate_limit.secondary_window.used_percent}
          resetAt={data.rate_limit.secondary_window.reset_at}
        />
      )}
      {data.code_review_rate_limit?.primary_window && (
        <WindowRow
          label="Code review"
          pct={data.code_review_rate_limit.primary_window.used_percent}
          limitReached={data.code_review_rate_limit.limit_reached}
        />
      )}
      {data.credits && (
        <Text size="1" tone="muted">
          Credits:{" "}
          {data.credits.unlimited
            ? "unlimited"
            : data.credits.has_credits
              ? `${data.credits.balance} remaining`
              : "none"}
        </Text>
      )}
    </div>
  );
}

function ProviderQuotaCard() {
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

  const rows: React.ReactNode[] = [];
  claudeInstances.forEach((p) =>
    rows.push(
      <ClaudeCodeInstanceRow
        key={`claude:${p.name}`}
        providerName={p.name}
        displayName={p.display_name}
      />,
    ),
  );
  codexInstances.forEach((p) =>
    rows.push(
      <OpenAICodexInstanceRow
        key={`codex:${p.name}`}
        providerName={p.name}
        displayName={p.display_name}
      />,
    ),
  );

  return (
    <div className={styles.card}>
      <Text size="1" weight="bold" tone="muted" className={styles.cardTitle}>
        PROVIDER QUOTAS
      </Text>
      {rows.map((row, idx) => (
        <React.Fragment key={idx}>
          {idx > 0 && <div className={styles.cardDivider} />}
          {row}
        </React.Fragment>
      ))}
    </div>
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
  const { data: integrationsData } =
    integrationsApi.useGetAllIntegrationsQuery(undefined);
  const { data: knowledgeData } = useGetKnowledgeGraphQuery(undefined);

  const providerCount =
    providersData?.providers.filter((p) => p.enabled).length ?? 0;
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
        <Text size="1" tone="danger">
          Failed to load stats
        </Text>
      </div>
    );
  }

  if (isLoading || !data) {
    if (compact) {
      return (
        <div className={styles.compactRow}>
          <Skeleton>
            <Text size="1">Loading stats...</Text>
          </Skeleton>
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
  const successRate =
    totals.total_calls > 0
      ? Math.round((totals.successful_calls / totals.total_calls) * 100)
      : 0;
  const successColor =
    successRate >= 95 ? "green" : successRate >= 80 ? "amber" : "red";
  const costStr = formatCost(totals.total_cost_usd);
  const failedCalls = totals.failed_calls;
  const cacheHitRate =
    totals.total_tokens > 0
      ? Math.round((totals.total_cache_read_tokens / totals.total_tokens) * 100)
      : 0;

  const dailyCostUsd =
    totals.total_cost_usd != null ? totals.total_cost_usd / 7 : 0;
  const hasUsageTracking = totals.total_calls > 0;

  if (compact) {
    return (
      <div className={styles.compactRow}>
        <Text size="1" tone="muted">
          {totals.total_conversations} chats ·{" "}
          {formatTokenCount(totals.total_tokens)} tok · {costStr}
          {totals.total_calls > 0 ? ` · ${successRate}% ok` : ""}
        </Text>
      </div>
    );
  }

  if (breakpoint === "narrow") {
    return (
      <div className={styles.narrowStats}>
        <Flex justify="between" align="center">
          <Text size="1" tone="muted">
            {totals.total_conversations} chats ·{" "}
            {formatTokenCount(totals.total_tokens)} tok
          </Text>
          <Text size="1" tone="muted">
            {costStr}
          </Text>
        </Flex>
        <Flex justify="between" align="center" gap="2">
          {totals.total_calls > 0 && (
            <Badge size="1" color={successColor} variant="soft">
              {successRate}% success
            </Badge>
          )}
          {providerCount > 0 && (
            <Text size="1" tone="muted">
              {providerCount} active providers
            </Text>
          )}
        </Flex>
        <SparklineChart days={by_day} />
      </div>
    );
  }

  const topModes = [...by_mode]
    .sort((a, b) => b.total_calls - a.total_calls)
    .slice(0, 3);
  const totalModeCalls = topModes.reduce((s, m) => s + m.total_calls, 0) || 1;

  return (
    <div className={styles.statsGrid} data-breakpoint={breakpoint}>
      <DefaultModelsCard />
      <ProviderQuotaCard />
      {/* Card 1: 7-Day Activity */}
      <div className={styles.card}>
        <Text size="1" weight="bold" tone="muted" className={styles.cardTitle}>
          7-DAY ACTIVITY
        </Text>

        <div className={styles.cardSection}>
          <Flex justify="between" align="center">
            <HoverStat label={`${totals.total_conversations} conversations`}>
              <Flex direction="column" gap="1">
                <Text size="2" weight="bold">
                  Conversations
                </Text>
                <Text size="1" tone="muted">
                  Total unique chat sessions in the last 7 days.
                </Text>
                <Text size="1">
                  {totals.total_messages_sent} user messages sent across all
                  chats.
                </Text>
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
              cache={
                totals.total_cache_read_tokens +
                totals.total_cache_creation_tokens
              }
            />
          </Flex>
          <HoverStat
            label={`${formatTokenCount(totals.total_tokens)} total tokens`}
          >
            <Flex direction="column" gap="1">
              <Text size="2" weight="bold">
                Token Breakdown
              </Text>
              <Text size="1">
                Prompt: {formatTokenCount(totals.total_prompt_tokens)}
              </Text>
              <Text size="1">
                Completion: {formatTokenCount(totals.total_completion_tokens)}
              </Text>
              <Text size="1">
                Cache read: {formatTokenCount(totals.total_cache_read_tokens)}
              </Text>
              <Text size="1">
                Cache created:{" "}
                {formatTokenCount(totals.total_cache_creation_tokens)}
              </Text>
              {cacheHitRate > 0 && (
                <Text size="1" tone="muted">
                  Cache hit rate: {cacheHitRate}% of tokens served from cache.
                </Text>
              )}
              {!hasUsageTracking && (
                <Text size="1" color="amber">
                  Note: Not all threads have tracked usage data.
                </Text>
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
                  <Text size="2" weight="bold">
                    LLM Call Success Rate
                  </Text>
                  <Text size="1" tone="muted">
                    Percentage of successful LLM API calls out of all attempts.
                    Failures include network errors, rate limits, and model
                    errors.
                  </Text>
                  <Text size="1">
                    {totals.successful_calls} succeeded / {totals.total_calls}{" "}
                    total calls
                  </Text>
                  {failedCalls > 0 && (
                    <Text size="1" tone="danger">
                      {failedCalls} failed calls (retries, timeouts, rate
                      limits)
                    </Text>
                  )}
                </Flex>
              </HoverStat>
              <Badge size="1" color={successColor} variant="soft">
                {successRate}%
              </Badge>
            </Flex>
          )}
          {totals.avg_duration_ms > 0 && (
            <HoverStat
              label={`Avg ${Math.round(totals.avg_duration_ms)}ms response`}
            >
              <Flex direction="column" gap="1">
                <Text size="2" weight="bold">
                  Average Response Time
                </Text>
                <Text size="1" tone="muted">
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
        <Text size="1" weight="bold" tone="muted" className={styles.cardTitle}>
          PROJECT PULSE
        </Text>

        <div className={styles.cardSection}>
          <Flex align="center" gap="3">
            {topModes.length > 0 && (
              <MiniDonut
                segments={topModes.map((m, i) => ({
                  value: m.total_calls,
                  color: [
                    "var(--rf-color-accent)",
                    "var(--rf-color-success)",
                    "var(--rf-color-warning)",
                    "var(--rf-color-accent-soft)",
                    "var(--rf-color-danger)",
                  ][i % 5],
                  label: m.mode,
                }))}
              />
            )}
            <Flex direction="column" gap="1" style={{ flex: 1 }}>
              <HoverStat label={`${by_mode.length} modes used`}>
                <Flex direction="column" gap="1">
                  <Text size="2" weight="bold">
                    Agent Modes
                  </Text>
                  <Text size="1" tone="muted">
                    Different modes determine which tools and prompts the AI
                    uses. Common modes: Agent (full tools), Explore (read-only),
                    Chat (no tools).
                  </Text>
                  {by_mode.map((m) => (
                    <Flex key={m.mode} justify="between" gap="2">
                      <Text size="1">{m.mode}</Text>
                      <Text size="1" tone="muted">
                        {m.total_calls} calls
                      </Text>
                    </Flex>
                  ))}
                </Flex>
              </HoverStat>
              {topModes.slice(0, 3).map((m) => {
                const pct = Math.round((m.total_calls / totalModeCalls) * 100);
                return (
                  <Text key={m.mode} size="1" tone="muted">
                    {m.mode} {pct}%
                  </Text>
                );
              })}
            </Flex>
          </Flex>
        </div>

        <div className={styles.cardDivider} />

        <div className={styles.cardSection}>
          <Flex align="center" gap="3">
            {by_model.length > 0 && (
              <MiniDonut
                segments={by_model.slice(0, 5).map((m, i) => ({
                  value: m.total_tokens,
                  color: [
                    "var(--rf-color-accent)",
                    "var(--rf-color-success)",
                    "var(--rf-color-warning)",
                    "var(--rf-color-accent-soft)",
                    "var(--rf-color-danger)",
                  ][i % 5],
                  label: m.model.split("/").pop() ?? m.model,
                }))}
              />
            )}
            <Flex direction="column" gap="1" style={{ flex: 1 }}>
              <HoverStat label={`${by_model.length} models used`}>
                <Flex direction="column" gap="1">
                  <Text size="2" weight="bold">
                    Model Usage
                  </Text>
                  <Text size="1" tone="muted">
                    Token usage across different LLM models in the last 7 days.
                  </Text>
                  {by_model.slice(0, 5).map((m) => (
                    <Flex key={m.model_id || m.model} justify="between" gap="2">
                      <Text size="1" truncate>
                        {m.model.split("/").pop() ?? m.model}
                      </Text>
                      <Text size="1" tone="muted">
                        {formatTokenCount(m.total_tokens)} tok
                      </Text>
                    </Flex>
                  ))}
                </Flex>
              </HoverStat>
              <ModelBars models={by_model} />
            </Flex>
          </Flex>
        </div>

        <div className={styles.cardDivider} />

        <div className={styles.cardSection}>
          <Flex align="center" gap="3">
            <MiniDonut
              segments={[
                {
                  value: providerCount,
                  color: "var(--rf-color-accent)",
                  label: "Providers",
                },
                {
                  value: integrationCount,
                  color: "var(--rf-color-success)",
                  label: "Integrations",
                },
                {
                  value: memoryCount,
                  color: "var(--rf-color-warning)",
                  label: "Memories",
                },
              ]}
            />
            <Flex direction="column" gap="1" style={{ flex: 1 }}>
              {providerCount > 0 && (
                <HoverStat label={`${providerCount} active providers`}>
                  <Flex direction="column" gap="1">
                    <Text size="2" weight="bold">
                      LLM Providers
                    </Text>
                    <Text size="1" tone="muted">
                      Enabled LLM providers (e.g. OpenAI, Anthropic, local
                      models).
                    </Text>
                    {totalModels > 0 && (
                      <Text size="1">
                        {totalModels} models available across all providers.
                      </Text>
                    )}
                  </Flex>
                </HoverStat>
              )}
              {integrationCount > 0 && (
                <HoverStat label={`${integrationCount} integrations`}>
                  <Flex direction="column" gap="1">
                    <Text size="2" weight="bold">
                      Integrations
                    </Text>
                    <Text size="1" tone="muted">
                      Connected tools and services: GitHub, Docker, databases,
                      MCP servers, etc.
                    </Text>
                  </Flex>
                </HoverStat>
              )}
              {memoryCount > 0 && (
                <HoverStat label={`${memoryCount} memories`}>
                  <Flex direction="column" gap="1">
                    <Text size="2" weight="bold">
                      Knowledge Memories
                    </Text>
                    <Text size="1" tone="muted">
                      Persistent knowledge entries the AI remembers across
                      sessions. Includes project patterns, decisions, and
                      learned preferences.
                    </Text>
                  </Flex>
                </HoverStat>
              )}
            </Flex>
          </Flex>
        </div>
      </div>

      {/* Card 3: Spending */}
      <div className={styles.card}>
        <Text size="1" weight="bold" tone="muted" className={styles.cardTitle}>
          SPENDING
        </Text>

        <div className={styles.cardSection}>
          <HoverStat label={`Total: ${costStr}`}>
            <Flex direction="column" gap="1">
              <Text size="2" weight="bold">
                7-Day Cost
              </Text>
              {totals.total_cost_usd != null && totals.total_cost_usd > 0 && (
                <Text size="1">USD: ${totals.total_cost_usd.toFixed(4)}</Text>
              )}
              <Text size="1" tone="muted">
                Cost is calculated per LLM API call based on token usage. Not
                all conversations may have tracked cost data.
              </Text>
            </Flex>
          </HoverStat>
          <HoverStat
            label={`Rate: ${
              dailyCostUsd > 0 ? formatRate(dailyCostUsd) : "free"
            }`}
          >
            <Flex direction="column" gap="1">
              <Text size="2" weight="bold">
                Daily Spend Rate
              </Text>
              <Text size="1" tone="muted">
                Average daily cost over the last 7 days. Actual daily spend
                varies based on usage patterns.
              </Text>
            </Flex>
          </HoverStat>
        </div>

        <div className={styles.cardDivider} />

        {top_conversations.length > 0 && (
          <div className={styles.cardSection}>
            <Text size="1" tone="muted">
              Top spenders:
            </Text>
            {top_conversations.slice(0, 3).map((conv: ConversationStats) => {
              const convCost = formatCost(conv.total_cost_usd);
              const shortModel =
                conv.model_id.split("/").pop() ?? conv.model_id;
              return (
                <Flex
                  key={conv.chat_id}
                  justify="between"
                  align="center"
                  gap="1"
                >
                  <Text size="1" truncate style={{ flex: 1, minWidth: 0 }}>
                    {shortModel}
                  </Text>
                  <Text size="1" tone="muted" style={{ flexShrink: 0 }}>
                    {formatTokenCount(conv.total_tokens)} tok · {convCost}
                  </Text>
                </Flex>
              );
            })}
          </div>
        )}

        <div className={styles.cardDivider} />

        <div className={styles.cardSection}>
          <HoverStat
            label={`${formatTokenCount(
              totals.total_prompt_tokens,
            )} prompt tokens`}
          >
            <Flex direction="column" gap="1">
              <Text size="2" weight="bold">
                Prompt Tokens
              </Text>
              <Text size="1" tone="muted">
                Tokens sent to the LLM (system prompt + conversation context +
                tool results). This is typically the largest cost component.
              </Text>
            </Flex>
          </HoverStat>
          <HoverStat
            label={`${formatTokenCount(
              totals.total_completion_tokens,
            )} completion tokens`}
          >
            <Flex direction="column" gap="1">
              <Text size="2" weight="bold">
                Completion Tokens
              </Text>
              <Text size="1" tone="muted">
                Tokens generated by the LLM (responses, tool calls, reasoning).
                Usually 3-5x more expensive per token than prompt tokens.
              </Text>
            </Flex>
          </HoverStat>
          {cacheHitRate > 0 && (
            <HoverStat label={`${cacheHitRate}% cache hit rate`}>
              <Flex direction="column" gap="1">
                <Text size="2" weight="bold">
                  Cache Efficiency
                </Text>
                <Text size="1" tone="muted">
                  Percentage of tokens served from provider cache (Anthropic
                  prompt caching, etc.). Cached tokens are significantly cheaper
                  than fresh computation.
                </Text>
                <Text size="1">
                  {formatTokenCount(totals.total_cache_read_tokens)} tokens read
                  from cache.
                </Text>
              </Flex>
            </HoverStat>
          )}
        </div>
      </div>
    </div>
  );
};
