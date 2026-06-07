import React, { useMemo } from "react";
import { HoverCard, Flex, Text, Badge } from "../LongTailPrimitives";
import {
  useGetClaudeCodeUsageQuery,
  useGetOpenAICodexUsageQuery,
  type ClaudeCodeUsageWindow,
  type OpenAICodexUsageWindow,
  type OpenAICodexRateLimit,
} from "../../services/refact/providers";
import { useCapsForToolUse, useGetConfiguredProvidersQuery } from "../../hooks";
import styles from "./UsageCounter.module.css";

const CircularUsage: React.FC<{
  pct: number;
  size?: number;
  strokeWidth?: number;
}> = ({ pct, size = 20, strokeWidth = 3 }) => {
  const clamped = Math.max(0, Math.min(pct, 100));
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const strokeDashoffset = circumference - (clamped / 100) * circumference;
  const fillClass =
    clamped >= 90
      ? styles.circularProgressFillOverflown
      : clamped >= 70
        ? styles.circularProgressFillWarning
        : styles.circularProgressFill;

  return (
    <svg width={size} height={size} className={styles.circularProgress}>
      <circle
        className={styles.circularProgressBg}
        cx={size / 2}
        cy={size / 2}
        r={radius}
        strokeWidth={strokeWidth}
      />
      <circle
        className={fillClass}
        cx={size / 2}
        cy={size / 2}
        r={radius}
        strokeWidth={strokeWidth}
        strokeDasharray={circumference}
        strokeDashoffset={strokeDashoffset}
        strokeLinecap="round"
      />
    </svg>
  );
};

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

const UsageRow: React.FC<{
  label: string;
  pct: number;
  resetAt?: string | null;
}> = ({ label, pct, resetAt }) => {
  const clamped = Math.max(0, Math.min(pct, 100));
  const color =
    clamped >= 90
      ? "var(--red-9)"
      : clamped >= 70
        ? "var(--orange-9)"
        : "var(--green-9)";
  const resetText = formatResetAt(resetAt);
  return (
    <Flex direction="column" gap="1">
      <Flex justify="between" align="center">
        <Text size="1" color="gray">
          {label}
        </Text>
        <Text size="1" color="gray">
          {Math.round(clamped)}% used{resetText ? ` · ${resetText}` : ""}
        </Text>
      </Flex>
      <div
        style={{
          height: "3px",
          width: "100%",
          borderRadius: "2px",
          background: "var(--gray-a4)",
          overflow: "hidden",
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
    </Flex>
  );
};

const ClaudeWindowRow: React.FC<{
  label: string;
  w: ClaudeCodeUsageWindow;
}> = ({ label, w }) => (
  <UsageRow label={label} pct={w.percent_used} resetAt={w.resets_at} />
);

const CodexWindowRow: React.FC<{
  label: string;
  w: OpenAICodexUsageWindow;
  limitReached?: boolean;
}> = ({ label, w, limitReached }) => {
  const resetText = formatResetAt(w.reset_at);
  return (
    <Flex direction="column" gap="1">
      <Flex justify="between" align="center">
        <Flex align="center" gap="1">
          <Text size="1" color="gray">
            {label}
          </Text>
          {limitReached && (
            <Badge color="red" size="1">
              Limit reached
            </Badge>
          )}
        </Flex>
        <Text size="1" color="gray">
          {Math.round(Math.max(0, Math.min(w.used_percent, 100)))}% used
          {resetText ? ` · ${resetText}` : ""}
        </Text>
      </Flex>
      <div
        style={{
          height: "3px",
          width: "100%",
          borderRadius: "2px",
          background: "var(--gray-a4)",
          overflow: "hidden",
        }}
      >
        <div
          style={{
            height: "100%",
            width: `${Math.max(0, Math.min(w.used_percent, 100))}%`,
            borderRadius: "2px",
            background:
              w.used_percent >= 90
                ? "var(--red-9)"
                : w.used_percent >= 70
                  ? "var(--orange-9)"
                  : "var(--green-9)",
            transition: "width 0.3s ease",
          }}
        />
      </div>
    </Flex>
  );
};

const RateLimitSection: React.FC<{
  rl: OpenAICodexRateLimit;
  primaryLabel: string;
  secondaryLabel: string;
}> = ({ rl, primaryLabel, secondaryLabel }) => (
  <>
    {rl.primary_window && (
      <CodexWindowRow
        label={primaryLabel}
        w={rl.primary_window}
        limitReached={rl.limit_reached}
      />
    )}
    {rl.secondary_window && (
      <CodexWindowRow label={secondaryLabel} w={rl.secondary_window} />
    )}
  </>
);

const ProviderIndicator: React.FC<{
  label: string;
  pct: number;
  children: React.ReactNode;
}> = ({ label, pct, children }) => (
  <HoverCard.Root openDelay={100}>
    <HoverCard.Trigger>
      <Flex align="center" gap="1" style={{ cursor: "default", opacity: 0.7 }}>
        <CircularUsage pct={pct} />
        <Text size="1" color="gray">
          {label}
        </Text>
      </Flex>
    </HoverCard.Trigger>
    <HoverCard.Content side="top" align="end" style={{ width: 280 }}>
      {children}
    </HoverCard.Content>
  </HoverCard.Root>
);

const ClaudeCodeQuotaPill: React.FC<{
  providerName: string;
  displayName: string;
}> = ({ providerName, displayName }) => {
  const { data: claudeUsage } = useGetClaudeCodeUsageQuery(
    { providerName },
    { pollingInterval: 30_000 },
  );

  const data = claudeUsage?.data;
  if (!data) return null;
  if (!data.five_hour && !data.seven_day) return null;

  const candidates = [
    data.five_hour?.percent_used,
    data.seven_day?.percent_used,
  ].filter((v): v is number => v != null);
  const pct = candidates.length > 0 ? Math.max(...candidates) : 0;

  return (
    <ProviderIndicator label={displayName} pct={pct}>
      <Flex direction="column" gap="3">
        <Text size="2" weight="medium">
          {displayName} quota
        </Text>
        {data.five_hour && (
          <ClaudeWindowRow label="Session (5 hour)" w={data.five_hour} />
        )}
        {data.seven_day && (
          <ClaudeWindowRow label="Weekly" w={data.seven_day} />
        )}
        {data.extra_usage && (
          <Flex justify="between" align="center">
            <Text size="1" color="gray">
              Extra usage
            </Text>
            <Text size="1" color="gray">
              {data.extra_usage.is_enabled ? "enabled" : "disabled"}
              {" · "}${data.extra_usage.used_credits.toFixed(2)} spent
              {typeof data.extra_usage.monthly_limit === "number"
                ? ` / $${data.extra_usage.monthly_limit.toFixed(0)} limit`
                : " / unlimited"}
            </Text>
          </Flex>
        )}
        <Text size="1" color="gray">
          Instance: {providerName}
        </Text>
      </Flex>
    </ProviderIndicator>
  );
};

const OpenAICodexQuotaPill: React.FC<{
  providerName: string;
  displayName: string;
}> = ({ providerName, displayName }) => {
  const { data: codexUsage } = useGetOpenAICodexUsageQuery(
    { providerName },
    { pollingInterval: 30_000 },
  );

  const data = codexUsage?.data;
  if (!data?.rate_limit) return null;

  const rl = data.rate_limit;
  const candidates = [
    rl.primary_window?.used_percent,
    rl.secondary_window?.used_percent,
  ].filter((v): v is number => v != null);
  const pct = candidates.length > 0 ? Math.max(...candidates) : 0;

  return (
    <ProviderIndicator label={displayName} pct={pct}>
      <Flex direction="column" gap="3">
        <Flex align="center" gap="2">
          <Text size="2" weight="medium">
            {displayName} quota
          </Text>
          {data.plan_type && (
            <Badge color="blue" size="1">
              {data.plan_type}
            </Badge>
          )}
        </Flex>
        <RateLimitSection
          rl={rl}
          primaryLabel="Session (5 hour)"
          secondaryLabel="Weekly"
        />
        {data.code_review_rate_limit?.primary_window && (
          <CodexWindowRow
            label="Code review (weekly)"
            w={data.code_review_rate_limit.primary_window}
            limitReached={data.code_review_rate_limit.limit_reached}
          />
        )}
        {data.credits && (
          <Flex justify="between" align="center">
            <Text size="1" color="gray">
              Credits
            </Text>
            <Text size="1" color="gray">
              {data.credits.unlimited
                ? "unlimited"
                : data.credits.has_credits
                  ? `${data.credits.balance} remaining`
                  : "none"}
            </Text>
          </Flex>
        )}
        <Text size="1" color="gray">
          Instance: {providerName}
        </Text>
      </Flex>
    </ProviderIndicator>
  );
};

/**
 * Renders a quota indicator only when the currently selected chat model belongs
 * to a Claude Code or OpenAI Codex provider instance. The displayed quota is
 * scoped to that exact provider instance (multi-account safe).
 */
export const ProviderUsageIndicator: React.FC = () => {
  const { currentModel } = useCapsForToolUse();
  const { data: providersData } = useGetConfiguredProvidersQuery();

  const selectedProvider = useMemo(() => {
    if (!currentModel || !providersData?.providers) return null;
    const slashIdx = currentModel.indexOf("/");
    if (slashIdx <= 0) return null;
    const instanceName = currentModel.slice(0, slashIdx);
    return providersData.providers.find((p) => p.name === instanceName) ?? null;
  }, [currentModel, providersData]);

  if (!selectedProvider) return null;

  if (selectedProvider.base_provider === "claude_code") {
    return (
      <Flex align="center" gap="2">
        <ClaudeCodeQuotaPill
          providerName={selectedProvider.name}
          displayName={selectedProvider.display_name}
        />
      </Flex>
    );
  }

  if (selectedProvider.base_provider === "openai_codex") {
    return (
      <Flex align="center" gap="2">
        <OpenAICodexQuotaPill
          providerName={selectedProvider.name}
          displayName={selectedProvider.display_name}
        />
      </Flex>
    );
  }

  return null;
};
