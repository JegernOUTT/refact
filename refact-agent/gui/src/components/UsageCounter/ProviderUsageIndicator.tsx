import React, { useMemo } from "react";
import { Flex, HoverCard, ScrollArea, Text } from "../LongTailPrimitives";
import { ProviderQuota, type ProviderQuotaTone } from "../ui";
import {
  type ProviderQuotaFact,
  type ProviderQuotaSnapshot,
  type ProviderQuotaWindow,
} from "../../services/refact/providers";
import { useGetProviderQuotaQuery } from "../../services/refact";
import { useCapsForToolUse, useGetConfiguredProvidersQuery } from "../../hooks";
import {
  clampPercent,
  formatLimitWindowSeconds,
  formatQuotaMeta,
  formatResetAfterSeconds,
  formatResetAt,
  formatUsagePercent,
} from "../../utils/providerQuota";
import styles from "./UsageCounter.module.css";
import { resolveSelectedProvider } from "./resolveSelectedProvider";

const CircularUsage: React.FC<{ pct?: number }> = ({ pct }) => {
  const size = 20;
  const strokeWidth = 3;
  const clamped = pct == null ? null : clampPercent(pct);
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const strokeDashoffset =
    clamped == null
      ? circumference
      : circumference - (clamped / 100) * circumference;
  const fillClass =
    clamped == null
      ? styles.circularProgressBg
      : clamped >= 90
        ? styles.circularProgressFillOverflown
        : clamped >= 70
          ? styles.circularProgressFillWarning
          : styles.circularProgressFill;

  return (
    <svg
      aria-hidden="true"
      className={styles.circularProgress}
      height={size}
      width={size}
    >
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
        strokeDasharray={circumference}
        strokeDashoffset={strokeDashoffset}
        strokeLinecap="round"
        strokeWidth={strokeWidth}
      />
    </svg>
  );
};

function quotaTone(
  usedPercent: number | null | undefined,
  fallback: ProviderQuotaTone = "accent",
): ProviderQuotaTone {
  if (usedPercent == null) return fallback;
  if (usedPercent >= 90) return "danger";
  if (usedPercent >= 70) return "warning";
  return "accent";
}

function formatAmount(value: number): string {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(
    value,
  );
}

function windowValue(window: ProviderQuotaWindow): string {
  if (window.used != null && window.limit != null) {
    return `${formatAmount(window.used)} / ${formatAmount(window.limit)}`;
  }
  if (window.remaining != null) {
    return `${formatAmount(window.remaining)} remaining`;
  }
  if (window.used_percent != null) {
    return formatUsagePercent(window.used_percent);
  }
  return "Not reported";
}

function windowMeta(window: ProviderQuotaWindow): string | undefined {
  const duration = formatLimitWindowSeconds(window.window_seconds);
  const meta = formatQuotaMeta([
    duration ? `${duration} window` : null,
    formatResetAfterSeconds(window.reset_after_seconds),
    formatResetAt(window.reset_at),
    window.status ?? null,
  ]);
  return meta || undefined;
}

function factValue(fact: ProviderQuotaFact): string {
  let value: string;
  if (fact.value == null) value = "Not reported";
  else if (typeof fact.value === "boolean") value = fact.value ? "Yes" : "No";
  else if (typeof fact.value === "number") value = formatAmount(fact.value);
  else value = fact.value;
  return fact.unit ? `${value} ${fact.unit}` : value;
}

function snapshotPercent(
  snapshot: ProviderQuotaSnapshot | undefined,
): number | undefined {
  if (!snapshot?.available) return undefined;
  const percentages = snapshot.windows
    .map((window) => window.used_percent)
    .filter((value): value is number => value != null);
  return percentages.length > 0 ? Math.max(...percentages) : undefined;
}

type ProviderQuotaIndicatorQuery = {
  data?: { quota: ProviderQuotaSnapshot };
  isError: boolean;
  isLoading: boolean;
};

export const SnapshotRows: React.FC<{ snapshot: ProviderQuotaSnapshot }> = ({
  snapshot,
}) => {
  const fallbackTone: ProviderQuotaTone = snapshot.stale ? "warning" : "accent";

  return (
    <Flex direction="column" gap="2">
      {snapshot.stale ? (
        <ProviderQuota
          label="Quota status"
          meta="Showing the last reported snapshot"
          tone="warning"
          value="Stale"
        />
      ) : null}
      {snapshot.error ? (
        <ProviderQuota
          label="Quota error"
          tone="danger"
          value={snapshot.error}
        />
      ) : null}
      {!snapshot.available ? (
        <ProviderQuota
          label="Quota status"
          meta={snapshot.error ?? "This provider did not report quota data"}
          tone={snapshot.error ? "danger" : "muted"}
          value="Unavailable"
        />
      ) : null}
      {snapshot.windows.map((window) => (
        <ProviderQuota
          key={`window:${window.id}`}
          label={window.label}
          meta={windowMeta(window)}
          tone={quotaTone(window.used_percent, fallbackTone)}
          usedPercent={window.used_percent ?? undefined}
          value={windowValue(window)}
        />
      ))}
      {snapshot.facts.map((fact) => (
        <ProviderQuota
          key={`fact:${fact.id}`}
          label={fact.label}
          tone={fallbackTone}
          value={factValue(fact)}
        />
      ))}
      {snapshot.available &&
      snapshot.windows.length === 0 &&
      snapshot.facts.length === 0 ? (
        <ProviderQuota
          label="Quota status"
          tone="muted"
          value="No quota details reported"
        />
      ) : null}
      <ProviderQuota label="Source" tone="muted" value={snapshot.source} />
    </Flex>
  );
};

export const ProviderUsageIndicatorContent: React.FC<{
  displayName: string;
  providerName: string;
  quotaQuery: ProviderQuotaIndicatorQuery;
}> = ({ displayName, providerName, quotaQuery }) => {
  const snapshot = quotaQuery.data?.quota;
  const percent = snapshotPercent(snapshot);
  const stateLabel = quotaQuery.isLoading
    ? "Loading quota"
    : quotaQuery.isError
      ? "Quota error"
      : snapshot?.error
        ? "Quota error"
        : snapshot?.stale
          ? "Stale quota"
          : snapshot?.available && percent != null
            ? `${Math.round(percent)}% used`
            : snapshot?.available
              ? "Usage unknown"
              : "Quota unavailable";

  return (
    <HoverCard.Root openDelay={100}>
      <HoverCard.Trigger asChild>
        <button
          aria-label={`${displayName}: ${stateLabel}`}
          className={styles.providerIndicatorTrigger}
          type="button"
        >
          <Flex align="center" gap="1">
            <CircularUsage pct={percent} />
            <Text color="gray" size="1">
              {displayName}
            </Text>
          </Flex>
        </button>
      </HoverCard.Trigger>
      <ScrollArea scrollbars="vertical" asChild>
        <HoverCard.Content
          align="end"
          maxHeight="50vh"
          maxWidth="280px"
          side="top"
        >
          <Flex direction="column" gap="2">
            <Text size="1" weight="bold">
              {displayName}
            </Text>
            {quotaQuery.isLoading ? (
              <ProviderQuota
                label="Quota status"
                tone="muted"
                value="Loading…"
              />
            ) : quotaQuery.isError ? (
              <ProviderQuota
                label="Quota status"
                meta="Could not load provider quota"
                tone="danger"
                value="Error"
              />
            ) : snapshot ? (
              <SnapshotRows snapshot={snapshot} />
            ) : (
              <ProviderQuota
                label="Quota status"
                meta="The provider returned no snapshot"
                tone="muted"
                value="Unavailable"
              />
            )}
            <ProviderQuota
              label="Provider instance"
              tone="muted"
              value={providerName}
            />
          </Flex>
        </HoverCard.Content>
      </ScrollArea>
    </HoverCard.Root>
  );
};

export const ProviderUsageIndicator: React.FC = () => {
  const { currentModel } = useCapsForToolUse();
  const { data: providersData } = useGetConfiguredProvidersQuery();
  const selectedProvider = useMemo(
    () => resolveSelectedProvider(currentModel, providersData?.providers),
    [currentModel, providersData],
  );
  const quotaQuery = useGetProviderQuotaQuery(
    { providerName: selectedProvider?.name ?? "" },
    { pollingInterval: 60_000, skip: selectedProvider == null },
  );

  if (!selectedProvider) return null;

  return (
    <ProviderUsageIndicatorContent
      displayName={selectedProvider.display_name}
      providerName={selectedProvider.name}
      quotaQuery={quotaQuery}
    />
  );
};
