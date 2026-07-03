import type {
  CodeIntelDetail,
  CodeIntelResponse,
} from "../../../services/refact/types";
import { formatPercent } from "../../StatsDashboard/utils/formatters";

export function isCodeIntelDetail<T>(
  response: CodeIntelResponse<T> | undefined,
): response is CodeIntelDetail {
  return (
    typeof response === "object" && response !== null && "detail" in response
  );
}

export function formatRatio(value: number, fractionDigits = 0): string {
  if (!Number.isFinite(value)) return "—";
  return formatPercent(value * 100, fractionDigits);
}

export function formatMaybeFixed(value: number, fractionDigits = 2): string {
  if (!Number.isFinite(value)) return "—";
  return value.toFixed(fractionDigits);
}

export function clampRatio(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(1, value));
}
