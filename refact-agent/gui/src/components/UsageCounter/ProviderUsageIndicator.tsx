import React, { useMemo } from "react";
import { HoverCard, Flex, Text, Badge } from "../LongTailPrimitives";
import {
  useGetClaudeCodeUsageQuery,
  useGetOpenAICodexUsageQuery,
  useGetOpenCodeUsageQuery,
  type ClaudeCodeUsageWindow,
  type OpenAICodexAdditionalRateLimit,
  type OpenAICodexUsageWindow,
  type OpenAICodexRateLimit,
  type OpenCodeUsageData,
} from "../../services/refact/providers";
import { useCapsForToolUse, useGetConfiguredProvidersQuery } from "../../hooks";
import styles from "./UsageCounter.module.css";
import {
  clampPercent,
  formatClaudeExtraUsage,
  formatCodexCreditsDetails,
  formatCodexCreditsSummary,
  formatCodexSpendControl,
  formatLimitWindowSeconds,
  formatNullableBool,
  formatQuotaMeta,
  formatResetAfterSeconds,
  formatResetAt,
  formatUsagePercent,
  formatWindowLabel,
  getClaudeUsageWindowRows,
} from "../../utils/providerQuota";

const CircularUsage: React.FC<{
  pct: number;
  size?: number;
  strokeWidth?: number;
}> = ({ pct, size = 20, strokeWidth = 3 }) => {
  const clamped = clampPercent(pct);
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

const usageColor = (pct: number): string => {
  if (pct >= 90) return "var(--rf-color-danger)";
  if (pct >= 70) return "var(--rf-color-warning)";
  return "var(--rf-color-success)";
};

const UsageRow: React.FC<{
  label: string;
  pct: number;
  resetAt?: string | null;
}> = ({ label, pct, resetAt }) => {
  const clamped = clampPercent(pct);
  const color = usageColor(clamped);
  const meta = formatQuotaMeta([
    formatUsagePercent(clamped),
    formatResetAt(resetAt),
  ]);
  return (
    <Flex direction="column" gap="1">
      <Flex justify="between" align="center">
        <Text size="1" color="gray">
          {label}
        </Text>
        <Text size="1" color="gray">
          {meta}
        </Text>
      </Flex>
      <div
        style={{
          height: "3px",
          width: "100%",
          borderRadius: "2px",
          background: "var(--rf-surface-2)",
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
  const clamped = clampPercent(w.used_percent);
  const windowText = formatLimitWindowSeconds(w.limit_window_seconds);
  const meta = formatQuotaMeta([
    formatUsagePercent(clamped),
    windowText ? `Window ${windowText}` : null,
    formatResetAfterSeconds(w.reset_after_seconds),
    formatResetAt(w.reset_at),
  ]);
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
          {meta}
        </Text>
      </Flex>
      <div
        style={{
          height: "3px",
          width: "100%",
          borderRadius: "2px",
          background: "var(--rf-surface-2)",
          overflow: "hidden",
        }}
      >
        <div
          style={{
            height: "100%",
            width: `${clamped}%`,
            borderRadius: "2px",
            background: usageColor(clamped),
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
    <Text size="1" color="gray">
      {formatQuotaMeta([
        `allowed ${formatNullableBool(rl.allowed)}`,
        `limit reached ${formatNullableBool(rl.limit_reached)}`,
      ])}
    </Text>
    {rl.primary_window && (
      <CodexWindowRow
        label={formatWindowLabel(
          primaryLabel,
          rl.primary_window.limit_window_seconds,
        )}
        w={rl.primary_window}
        limitReached={rl.limit_reached}
      />
    )}
    {rl.secondary_window && (
      <CodexWindowRow
        label={formatWindowLabel(
          secondaryLabel,
          rl.secondary_window.limit_window_seconds,
        )}
        w={rl.secondary_window}
      />
    )}
    {!rl.primary_window && !rl.secondary_window && (
      <Text size="1" color="gray">
        No active windows reported.
      </Text>
    )}
  </>
);

const AdditionalRateLimitSummary: React.FC<{
  limits: OpenAICodexAdditionalRateLimit[];
}> = ({ limits }) => (
  <Flex direction="column" gap="1">
    <Text size="1" color="gray">
      Additional quotas: {limits.length}
    </Text>
    {limits.slice(0, 3).map((limit, index) => (
      <Text
        key={`${limit.limit_name ?? "quota"}-${index}`}
        size="1"
        color="gray"
      >
        {formatQuotaMeta([
          limit.limit_name ?? "Additional quota",
          limit.metered_feature ?? null,
        ])}
      </Text>
    ))}
  </Flex>
);

const ProviderIndicator: React.FC<{
  label: string;
  pct: number;
  children: React.ReactNode;
}> = ({ label, pct, children }) => (
  <HoverCard.Root openDelay={100}>
    <HoverCard.Trigger asChild>
      <Flex align="center" gap="1" className={styles.providerIndicatorTrigger}>
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
  const windowRows = getClaudeUsageWindowRows(data);
  if (windowRows.length === 0 && !data.extra_usage) return null;

  const candidates = windowRows.map((row) => row.window.percent_used);
  const pct = candidates.length > 0 ? Math.max(...candidates) : 0;

  return (
    <ProviderIndicator label={displayName} pct={pct}>
      <Flex direction="column" gap="3">
        <Text size="2" weight="medium">
          {displayName} quota
        </Text>
        {windowRows.map(({ key, label, window }) => (
          <ClaudeWindowRow key={key} label={label} w={window} />
        ))}
        {windowRows.length === 0 && (
          <Text size="1" color="gray">
            Quota windows not reported.
          </Text>
        )}
        {data.extra_usage && (
          <Flex justify="between" align="center" gap="2">
            <Text size="1" color="gray">
              Extra usage
            </Text>
            <Text size="1" color="gray" align="right">
              {formatClaudeExtraUsage(data.extra_usage)}
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
          primaryLabel="Session"
          secondaryLabel="Secondary"
        />
        {data.additional_rate_limits?.length ? (
          <AdditionalRateLimitSummary limits={data.additional_rate_limits} />
        ) : (
          <Text size="1" color="gray">
            Additional quotas not reported.
          </Text>
        )}
        <Flex direction="column" gap="1">
          <Text size="1" color="gray">
            Code review quota
          </Text>
          {data.code_review_rate_limit ? (
            <RateLimitSection
              rl={data.code_review_rate_limit}
              primaryLabel="Code review"
              secondaryLabel="Code review secondary"
            />
          ) : (
            <Text size="1" color="gray">
              Not reported.
            </Text>
          )}
        </Flex>
        {data.credits && (
          <Flex direction="column" gap="1">
            <Flex justify="between" align="center" gap="2">
              <Text size="1" color="gray">
                Credits
              </Text>
              <Text size="1" color="gray" align="right">
                {formatCodexCreditsSummary(data.credits)}
              </Text>
            </Flex>
            {formatCodexCreditsDetails(data.credits) && (
              <Text size="1" color="gray">
                {formatCodexCreditsDetails(data.credits)}
              </Text>
            )}
          </Flex>
        )}
        {data.spend_control && (
          <Text size="1" color="gray">
            Spend control: {formatCodexSpendControl(data.spend_control)}
          </Text>
        )}
        <Text size="1" color="gray">
          Instance: {providerName}
        </Text>
      </Flex>
    </ProviderIndicator>
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

const OpenCodeQuotaPill: React.FC<{
  providerName: string;
  displayName: string;
}> = ({ providerName, displayName }) => {
  const { data: openCodeUsage } = useGetOpenCodeUsageQuery(
    { providerName },
    { pollingInterval: 30_000 },
  );

  const data = openCodeUsage?.data;
  if (!data) return null;

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
  if (windows.length === 0 && typeof data.balance !== "number") return null;

  const pct = Math.max(0, ...windows.map(({ window }) => window.used_percent));

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
        {data.workspace_id && (
          <Text size="1" color="gray">
            Workspace: {data.workspace_id}
          </Text>
        )}
        {typeof data.balance === "number" && (
          <Text size="1" color="gray">
            Zen balance:{" "}
            {data.balance.toLocaleString(undefined, {
              maximumFractionDigits: 2,
            })}
          </Text>
        )}
        {windows.length > 0 ? (
          <Flex direction="column" gap="2">
            {windows.map(({ key, label, window }) => (
              <CodexWindowRow
                key={key}
                label={formatWindowLabel(label, window.limit_window_seconds)}
                w={window}
                limitReached={window.status === "rate-limited"}
              />
            ))}
          </Flex>
        ) : (
          <Text size="1" color="gray">
            Quota windows not reported.
          </Text>
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
 * to a provider instance with subscription quota data. The displayed quota is
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

  if (selectedProvider.base_provider === "opencode") {
    return (
      <Flex align="center" gap="2">
        <OpenCodeQuotaPill
          providerName={selectedProvider.name}
          displayName={selectedProvider.display_name}
        />
      </Flex>
    );
  }

  return null;
};
