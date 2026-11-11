import type {
  LiteLLMFeatureState,
  LiteLLMScope,
} from "../../../../services/refact/providers";

export type LiteLLMTone =
  | "default"
  | "accent"
  | "success"
  | "warning"
  | "danger"
  | "muted";

export function formatUsd(value: number, currency?: string | null): string {
  if (!Number.isFinite(value)) return "Unknown";
  const abs = Math.abs(value);
  const maxFractionDigits = abs === 0 ? 2 : abs < 0.01 ? 8 : abs < 1 ? 4 : 2;
  const code =
    currency && /^[A-Za-z]{3}$/.test(currency) ? currency.toUpperCase() : "USD";
  try {
    return new Intl.NumberFormat(undefined, {
      style: "currency",
      currency: code,
      minimumFractionDigits: 2,
      maximumFractionDigits: maxFractionDigits,
    }).format(value);
  } catch {
    return `${value.toFixed(2)} ${code}`;
  }
}

export function formatCount(value: number | null | undefined): string {
  if (value === null || value === undefined || !Number.isFinite(value)) {
    return "Not reported";
  }
  return value.toLocaleString(undefined, { maximumFractionDigits: 0 });
}

export function formatWhen(value: string | null | undefined): string | null {
  if (!value) return null;
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return null;
  return parsed.toLocaleString();
}

export function formatLimit(
  holder: object,
  key: string,
  value: number | null | undefined,
): string {
  if (!(key in holder) || value === undefined) return "Not reported";
  if (value === null) return "No limit";
  return formatCount(value);
}

export const FEATURE_STATE_LABEL: Record<LiteLLMFeatureState, string> = {
  ok: "OK",
  unsupported: "Unsupported",
  permission_denied: "Permission denied",
  auth_required: "Auth required",
  not_configured: "Not configured",
  unavailable: "Unavailable",
  malformed_response: "Unreadable response",
  unknown: "Unknown",
};

export function featureStateTone(state: LiteLLMFeatureState): LiteLLMTone {
  switch (state) {
    case "ok":
      return "success";
    case "permission_denied":
    case "malformed_response":
      return "danger";
    case "auth_required":
    case "not_configured":
    case "unavailable":
      return "warning";
    default:
      return "muted";
  }
}

export function featureStateExplanation(
  state: LiteLLMFeatureState,
  feature: string,
): string {
  switch (state) {
    case "ok":
      return "";
    case "unsupported":
      return `This LiteLLM deployment does not expose ${feature}.`;
    case "permission_denied":
      return `The configured key is not permitted to read ${feature}. This is a permission limit, not zero usage.`;
    case "auth_required":
      return `Authentication is required to read ${feature}.`;
    case "not_configured":
      return `${
        feature.charAt(0).toUpperCase() + feature.slice(1)
      } is not configured. Save an endpoint and API key first.`;
    case "unavailable":
      return `${
        feature.charAt(0).toUpperCase() + feature.slice(1)
      } is temporarily unavailable.`;
    case "malformed_response":
      return `The gateway returned an unreadable response for ${feature}.`;
    default:
      return `The status of ${feature} is unknown.`;
  }
}

export const SCOPE_LABEL: Record<LiteLLMScope, string> = {
  key: "Key",
  user: "User",
  team: "Team",
};

export function isSafeExternalUrl(value: string | null | undefined): boolean {
  if (!value) return false;
  try {
    const url = new URL(value);
    return (
      (url.protocol === "https:" || url.protocol === "http:") &&
      url.username === "" &&
      url.password === ""
    );
  } catch {
    return false;
  }
}

export function localDateString(date: Date): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, "0");
  const d = String(date.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

export const MAX_USAGE_RANGE_DAYS = 92;

export function validateDateRange(start: string, end: string): string | null {
  const iso = /^\d{4}-\d{2}-\d{2}$/;
  if (!iso.test(start) || !iso.test(end)) return "Choose a start and end date.";
  const s = Date.parse(`${start}T00:00:00Z`);
  const e = Date.parse(`${end}T00:00:00Z`);
  if (Number.isNaN(s) || Number.isNaN(e)) return "Choose valid dates.";
  if (s > e) return "Start date must not be after the end date.";
  if ((e - s) / 86_400_000 + 1 > MAX_USAGE_RANGE_DAYS) {
    return `Date range is limited to ${MAX_USAGE_RANGE_DAYS} days.`;
  }
  return null;
}
