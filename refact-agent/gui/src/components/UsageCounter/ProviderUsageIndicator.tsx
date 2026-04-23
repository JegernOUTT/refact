import React, { useEffect, useState } from "react";
import { HoverCard, Flex, Text, Badge } from "@radix-ui/themes";
import {
  useGetClaudeCodeUsageQuery,
  useGetOpenAICodexUsageQuery,
  type ClaudeCodeUsageResponse,
  type ClaudeCodeUsageWindow,
  type OpenAICodexUsageResponse,
  type OpenAICodexUsageWindow,
  type OpenAICodexRateLimit,
} from "../../services/refact/providers";

// Module-level cache: survives component unmounts (page navigation, chat switching)
let lastKnownClaudeUsage: ClaudeCodeUsageResponse | undefined;
let lastKnownCodexUsage: OpenAICodexUsageResponse | undefined;
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

export const ProviderUsageIndicator: React.FC = () => {
  const { data: claudeUsage } = useGetClaudeCodeUsageQuery(undefined, {
    pollingInterval: 30_000,
  });

  const { data: codexUsage } = useGetOpenAICodexUsageQuery(undefined, {
    pollingInterval: 30_000,
  });

  // Local render-trigger: module-level cache doesn't cause re-renders on its own
  const [, forceUpdate] = useState(0);

  const hasLiveClaudeData = !!(
    claudeUsage?.data &&
    (claudeUsage.data.five_hour ?? claudeUsage.data.seven_day)
  );
  const hasLiveCodexData = !!codexUsage?.data?.rate_limit;

  useEffect(() => {
    if (hasLiveClaudeData) {
      lastKnownClaudeUsage = claudeUsage;
      forceUpdate((n) => n + 1);
    }
  }, [hasLiveClaudeData, claudeUsage]);

  useEffect(() => {
    if (hasLiveCodexData) {
      lastKnownCodexUsage = codexUsage;
      forceUpdate((n) => n + 1);
    }
  }, [hasLiveCodexData, codexUsage]);

  // Prefer live data; fall back to module-level cache (survives unmount/remount)
  const stickyClaudeUsage = hasLiveClaudeData ? claudeUsage : lastKnownClaudeUsage;
  const stickyCodexUsage = hasLiveCodexData ? codexUsage : lastKnownCodexUsage;

  const hasClaudeData = !!(
    stickyClaudeUsage?.data &&
    (stickyClaudeUsage.data.five_hour ?? stickyClaudeUsage.data.seven_day)
  );
  const hasCodexData = !!stickyCodexUsage?.data?.rate_limit;

  if (!hasClaudeData && !hasCodexData) return null;

  const claudeData = stickyClaudeUsage?.data;
  const codexData = stickyCodexUsage?.data;

  let claudePct = 0;
  if (hasClaudeData && claudeData) {
    const candidates = [
      claudeData.five_hour?.percent_used,
      claudeData.seven_day?.percent_used,
    ].filter((v): v is number => v != null);
    if (candidates.length > 0) claudePct = Math.max(...candidates);
  }

  let codexPct = 0;
  if (hasCodexData && codexData?.rate_limit) {
    const rl = codexData.rate_limit;
    const candidates = [
      rl.primary_window?.used_percent,
      rl.secondary_window?.used_percent,
    ].filter((v): v is number => v != null);
    if (candidates.length > 0) codexPct = Math.max(...candidates);
  }

  return (
    <Flex align="center" gap="2">
      {hasClaudeData && claudeData && (
        <ProviderIndicator label="Claude" pct={claudePct}>
          <Flex direction="column" gap="3">
            <Text size="2" weight="medium">
              Claude Code quota
            </Text>
            {claudeData.five_hour && (
              <ClaudeWindowRow
                label="Session (5 hour)"
                w={claudeData.five_hour}
              />
            )}
            {claudeData.seven_day && (
              <ClaudeWindowRow label="Weekly" w={claudeData.seven_day} />
            )}
            {claudeData.extra_usage && (
              <Flex justify="between" align="center">
                <Text size="1" color="gray">
                  Extra usage
                </Text>
                <Text size="1" color="gray">
                  {claudeData.extra_usage.is_enabled ? "enabled" : "disabled"}
                  {" · "}${claudeData.extra_usage.used_credits.toFixed(2)} spent
                  {typeof claudeData.extra_usage.monthly_limit === "number"
                    ? ` / $${claudeData.extra_usage.monthly_limit.toFixed(
                        0,
                      )} limit`
                    : " / unlimited"}
                </Text>
              </Flex>
            )}
          </Flex>
        </ProviderIndicator>
      )}

      {hasCodexData && codexData && (
        <ProviderIndicator label="Codex" pct={codexPct}>
          <Flex direction="column" gap="3">
            <Flex align="center" gap="2">
              <Text size="2" weight="medium">
                OpenAI Codex quota
              </Text>
              {codexData.plan_type && (
                <Badge color="blue" size="1">
                  {codexData.plan_type}
                </Badge>
              )}
            </Flex>
            {codexData.rate_limit && (
              <RateLimitSection
                rl={codexData.rate_limit}
                primaryLabel="Session (5 hour)"
                secondaryLabel="Weekly"
              />
            )}
            {codexData.code_review_rate_limit?.primary_window && (
              <CodexWindowRow
                label="Code review (weekly)"
                w={codexData.code_review_rate_limit.primary_window}
                limitReached={codexData.code_review_rate_limit.limit_reached}
              />
            )}
            {codexData.credits && (
              <Flex justify="between" align="center">
                <Text size="1" color="gray">
                  Credits
                </Text>
                <Text size="1" color="gray">
                  {codexData.credits.unlimited
                    ? "unlimited"
                    : codexData.credits.has_credits
                      ? `${codexData.credits.balance} remaining`
                      : "none"}
                </Text>
              </Flex>
            )}
          </Flex>
        </ProviderIndicator>
      )}
    </Flex>
  );
};
