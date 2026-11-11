import React, { useMemo } from "react";
import { Popover } from "../ui";
import {
  ComposerTrigger,
  KitBadge,
  KitDivider,
  KitHeader,
  KitKeyValue,
  KitNote,
  KitPanel,
  KitProgress,
  KitStaticRow,
  kit,
} from "../ComposerKit";
import {
  type ProviderQuotaFact,
  type ProviderQuotaSnapshot,
  type ProviderQuotaWindow,
} from "../../services/refact/providers";
import { useGetProviderQuotaQuery } from "../../services/refact";
import { formatUsd } from "../../features/Providers/ProviderForm/LiteLLM/litellmFormat";
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
import { useHoverPopover } from "./useHoverPopover";

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
      viewBox={`0 0 ${size} ${size}`}
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
): "warning" | "danger" | undefined {
  if (usedPercent == null) return undefined;
  if (usedPercent >= 90) return "danger";
  if (usedPercent >= 70) return "warning";
  return undefined;
}

function formatAmount(value: number): string {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(
    value,
  );
}

function windowValue(
  window: ProviderQuotaWindow,
  money: (value: number) => string = formatAmount,
): string {
  if (window.used != null && window.limit != null) {
    return `${money(window.used)} / ${money(window.limit)}`;
  }
  if (window.remaining != null) {
    return `${money(window.remaining)} remaining`;
  }
  if (window.used_percent != null) {
    return formatUsagePercent(window.used_percent);
  }
  return "Not reported";
}

function sentenceCase(label: string): string {
  return label.charAt(0).toUpperCase() + label.slice(1);
}

function windowMeta(window: ProviderQuotaWindow): string | undefined {
  const duration = formatLimitWindowSeconds(window.window_seconds);
  const resetIn = formatResetAfterSeconds(window.reset_after_seconds);
  const resetAt = formatResetAt(window.reset_at);
  const reset =
    resetIn && resetAt
      ? `${resetIn} (${resetAt.replace(/^Resets\s+/i, "")})`
      : resetIn ?? resetAt;
  const meta = formatQuotaMeta([
    duration ? `${duration} window` : null,
    reset,
    window.status ?? null,
  ]);
  return meta || undefined;
}

function factValue(fact: ProviderQuotaFact, preciseUsd = false): string {
  if (fact.value == null) return "Not reported";
  if (preciseUsd && typeof fact.value === "number" && fact.unit === "USD") {
    return formatUsd(fact.value);
  }
  let value: string;
  if (typeof fact.value === "boolean") value = fact.value ? "Yes" : "No";
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
}) => (
  <>
    {snapshot.stale ? (
      <KitKeyValue
        label="Quota status"
        value={<KitBadge tone="warning">Stale</KitBadge>}
      />
    ) : null}
    {snapshot.error ? (
      <KitStaticRow tone="danger" label={snapshot.error} />
    ) : null}
    {!snapshot.available ? (
      <>
        <KitKeyValue label="Quota status" value="Unavailable" />
        {snapshot.error ? null : (
          <KitNote>This provider did not report quota data</KitNote>
        )}
      </>
    ) : null}
    {snapshot.windows.map((window) => {
      const meta = windowMeta(window);
      const money = snapshot.source === "litellm" ? formatUsd : formatAmount;
      return (
        <React.Fragment key={`window:${window.id}`}>
          <KitKeyValue
            label={sentenceCase(window.label)}
            value={windowValue(window, money)}
          />
          {window.used_percent != null ? (
            <KitProgress
              value={clampPercent(window.used_percent)}
              tone={quotaTone(window.used_percent)}
              label={window.label}
            />
          ) : null}
          {meta ? <KitNote>{meta}</KitNote> : null}
        </React.Fragment>
      );
    })}
    {snapshot.facts.map((fact) => (
      <KitKeyValue
        key={`fact:${fact.id}`}
        label={sentenceCase(fact.label)}
        value={factValue(fact, snapshot.source === "litellm")}
      />
    ))}
    {snapshot.available &&
    snapshot.windows.length === 0 &&
    snapshot.facts.length === 0 ? (
      <KitStaticRow tone="muted" label="No quota details reported" />
    ) : null}
    <KitDivider />
    <KitKeyValue label="Source" value={snapshot.source} />
  </>
);

export const ProviderUsageIndicatorContent: React.FC<{
  displayName: string;
  providerName: string;
  quotaQuery: ProviderQuotaIndicatorQuery;
  onHoldChange?: (held: boolean) => void;
}> = ({ displayName, providerName, quotaQuery, onHoldChange }) => {
  const hover = useHoverPopover(onHoldChange);
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
    <Popover
      responsive={false}
      open={hover.open}
      onOpenChange={hover.onOpenChange}
    >
      <Popover.Trigger asChild {...hover.triggerProps}>
        <ComposerTrigger aria-label={`${displayName}: ${stateLabel}`}>
          <CircularUsage pct={percent} />
          <span className={kit.label} data-shrink-at="1">
            {displayName}
          </span>
        </ComposerTrigger>
      </Popover.Trigger>
      <Popover.Content
        className={kit.popover}
        align="end"
        maxHeight="50vh"
        maxWidth="280px"
        side="top"
        {...hover.contentProps}
      >
        <KitPanel>
          <KitHeader
            title={displayName}
            meta={percent != null ? formatUsagePercent(percent) : undefined}
            metaStrong
          />
          {quotaQuery.isLoading ? (
            <KitStaticRow tone="muted" label="Loading…" />
          ) : quotaQuery.isError ? (
            <KitStaticRow tone="danger" label="Could not load provider quota" />
          ) : snapshot ? (
            <SnapshotRows snapshot={snapshot} />
          ) : (
            <KitStaticRow
              tone="muted"
              label="The provider returned no snapshot"
            />
          )}
          <KitKeyValue label="Provider instance" value={providerName} />
        </KitPanel>
      </Popover.Content>
    </Popover>
  );
};

export const ProviderUsageIndicator: React.FC<{
  onHoldChange?: (held: boolean) => void;
}> = ({ onHoldChange }) => {
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
      onHoldChange={onHoldChange}
    />
  );
};
