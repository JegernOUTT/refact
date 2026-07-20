import type {
  ClaudeCodeExtraUsage,
  ClaudeCodeScopedUsageWindow,
  ClaudeCodeUsageData,
  ClaudeCodeUsageWindow,
  OpenAICodexCredits,
  OpenAICodexSpendControl,
} from "../services/refact/providers";

const CLAUDE_USAGE_WINDOWS = [
  { key: "five_hour", label: "Current session" },
  { key: "seven_day", label: "Current week — all models" },
  { key: "seven_day_sonnet", label: "Current week — Sonnet" },
  { key: "seven_day_opus", label: "Current week — Opus" },
  { key: "seven_day_oauth_apps", label: "Current week — OAuth apps" },
  { key: "seven_day_cowork", label: "Current week — cowork" },
  { key: "seven_day_omelette", label: "Current week — Omelette" },
] as const;

export type ClaudeUsageWindowRow = {
  key: string;
  label: string;
  window: ClaudeCodeUsageWindow;
};

function isClaudeUsageWindow(window: unknown): window is ClaudeCodeUsageWindow {
  if (typeof window !== "object" || window === null) return false;
  const candidate = window as Partial<ClaudeCodeUsageWindow>;
  return (
    typeof candidate.percent_used === "number" &&
    Number.isFinite(candidate.percent_used)
  );
}

function isClaudeScopedUsageWindow(
  value: unknown,
): value is ClaudeCodeScopedUsageWindow {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<ClaudeCodeScopedUsageWindow>;
  return (
    typeof candidate.label === "string" &&
    (candidate.model_id == null || typeof candidate.model_id === "string") &&
    isClaudeUsageWindow(candidate.window)
  );
}

export function getClaudeUsageWindowRows(
  data: ClaudeCodeUsageData,
): ClaudeUsageWindowRow[] {
  const rows: ClaudeUsageWindowRow[] = [];
  const scopedIdentities = new Set<string>();

  const appendScopedWindows = () => {
    const scopedWindows = Array.isArray(data.scoped_windows)
      ? data.scoped_windows
      : [];
    for (const scoped of scopedWindows) {
      if (!isClaudeScopedUsageWindow(scoped)) continue;
      const label = scoped.label.trim();
      if (!label) continue;
      const modelId = scoped.model_id?.trim();
      const identity = (modelId ? modelId : label).toLocaleLowerCase();
      if (scopedIdentities.has(identity)) continue;
      scopedIdentities.add(identity);
      rows.push({
        key: `scoped:${identity}`,
        label: `Current week — ${label}`,
        window: scoped.window,
      });
    }
  };

  for (const { key, label } of CLAUDE_USAGE_WINDOWS) {
    const window = data[key];
    if (isClaudeUsageWindow(window)) {
      rows.push({ key, label, window });
    }
    if (key === "seven_day") appendScopedWindows();
  }

  return rows;
}

export function clampPercent(value: number): number {
  return Math.max(0, Math.min(value, 100));
}

export function formatResetAt(
  resetAt: string | null | undefined,
): string | null {
  if (!resetAt) return null;
  const trimmed = resetAt.trim();
  if (!trimmed) return null;
  const date = new Date(resetAt);
  if (Number.isNaN(date.getTime())) {
    return /^resets?\b/i.test(trimmed) ? trimmed : `Resets ${trimmed}`;
  }
  return `Resets ${date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  })}`;
}

export function formatLimitWindowSeconds(
  seconds: number | null | undefined,
): string | null {
  if (
    typeof seconds !== "number" ||
    !Number.isFinite(seconds) ||
    seconds <= 0
  ) {
    return null;
  }

  if (seconds % 86_400 === 0) {
    const days = seconds / 86_400;
    return days === 1 ? "1 day" : `${days} days`;
  }

  if (seconds % 3_600 === 0) {
    const hours = seconds / 3_600;
    return hours === 1 ? "1 hour" : `${hours} hours`;
  }

  if (seconds % 60 === 0) {
    const minutes = seconds / 60;
    return minutes === 1 ? "1 minute" : `${minutes} minutes`;
  }

  return seconds === 1 ? "1 second" : `${seconds} seconds`;
}

export function formatResetAfterSeconds(
  seconds: number | null | undefined,
): string | null {
  const duration = formatLimitWindowSeconds(seconds);
  return duration ? `Resets in ${duration}` : null;
}

export function formatUsagePercent(value: number): string {
  return `${Math.round(clampPercent(value))}% used`;
}

export function formatQuotaMeta(parts: (string | null | undefined)[]): string {
  return parts.filter((part): part is string => Boolean(part)).join(" · ");
}

function formatCreditAmount(value: number): string {
  return value.toLocaleString(undefined, {
    maximumFractionDigits: 2,
  });
}

export function formatCurrencyAmount(
  value: number,
  currency: string | null | undefined,
): string {
  if (!currency) return formatCreditAmount(value);
  try {
    return new Intl.NumberFormat(undefined, {
      style: "currency",
      currency,
      maximumFractionDigits: 2,
    }).format(value);
  } catch {
    return `${formatCreditAmount(value)} ${currency}`;
  }
}

export function formatWindowLabel(
  fallback: string,
  seconds: number | null | undefined,
): string {
  const duration = formatLimitWindowSeconds(seconds);
  return duration ? `${duration} window` : fallback;
}

export function formatNullableBool(
  value: boolean | null | undefined,
  trueLabel = "yes",
  falseLabel = "no",
  nullLabel = "not reported",
): string {
  if (value === true) return trueLabel;
  if (value === false) return falseLabel;
  return nullLabel;
}

export function formatNumberPair(
  values: number[] | null | undefined,
): string | null {
  if (!values || values.length === 0) return null;
  return values.map(formatCreditAmount).join(" / ");
}

export function formatCodexCreditsSummary(credits: OpenAICodexCredits): string {
  if (credits.unlimited) return "unlimited";
  if (credits.has_credits) {
    return `${formatCreditAmount(credits.balance)} remaining`;
  }
  return `${formatCreditAmount(credits.balance)} balance · no credits`;
}

export function formatCodexCreditsDetails(
  credits: OpenAICodexCredits,
): string | null {
  const details = [
    typeof credits.granted === "number"
      ? `${formatCreditAmount(credits.granted)} granted`
      : null,
    typeof credits.used === "number"
      ? `${formatCreditAmount(credits.used)} used`
      : null,
    typeof credits.overage_limit_reached === "boolean"
      ? `overage ${credits.overage_limit_reached ? "reached" : "not reached"}`
      : null,
    formatNumberPair(credits.approx_cloud_messages)
      ? `cloud approx ${formatNumberPair(credits.approx_cloud_messages)}`
      : null,
    formatNumberPair(credits.approx_local_messages)
      ? `local approx ${formatNumberPair(credits.approx_local_messages)}`
      : null,
    formatResetAt(credits.reset_at),
  ];
  return formatQuotaMeta(details) || null;
}

export function formatClaudeExtraUsage(
  extraUsage: ClaudeCodeExtraUsage,
): string {
  const spent =
    typeof extraUsage.used_credits === "number"
      ? `${formatCurrencyAmount(
          extraUsage.used_credits,
          extraUsage.currency,
        )} spent`
      : "spent not reported";
  const limit =
    typeof extraUsage.monthly_limit === "number"
      ? `${formatCurrencyAmount(
          extraUsage.monthly_limit,
          extraUsage.currency,
        )} limit`
      : extraUsage.is_enabled
        ? "unlimited"
        : "limit not reported";

  return formatQuotaMeta([
    extraUsage.is_enabled ? "enabled" : "disabled",
    extraUsage.disabled_reason ?? null,
    spent,
    limit,
    typeof extraUsage.utilization === "number"
      ? formatUsagePercent(extraUsage.utilization)
      : null,
  ]);
}

export function formatCodexSpendControl(
  spendControl: OpenAICodexSpendControl,
): string {
  return formatQuotaMeta([
    `reached ${formatNullableBool(spendControl.reached)}`,
    typeof spendControl.individual_limit === "number"
      ? `individual limit ${formatCreditAmount(spendControl.individual_limit)}`
      : "individual limit not reported",
  ]);
}
