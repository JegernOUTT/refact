import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import { redactBuddyFrontendErrorText } from "../../features/Buddy/reportBuddyFrontendError";

export function backgroundAgentErrorText(
  error: unknown,
  token?: string | null,
): string {
  const fields =
    typeof error === "object" && error !== null
      ? (error as Record<string, unknown>)
      : {};
  const data = fields.data;
  const details =
    typeof data === "object" && data !== null
      ? (data as Record<string, unknown>)
      : {};
  const text = [
    details.detail,
    details.message,
    details.error,
    data,
    fields.detail,
    fields.message,
    fields.error,
    error,
  ].find(
    (value): value is string =>
      typeof value === "string" && value.trim().length > 0,
  );
  const message = text ?? "Agent action failed.";
  return redactBuddyFrontendErrorText(
    token ? message.split(token).join("[REDACTED]") : message,
  )
    .replace(/[A-Za-z0-9_+/=-]{32,}/gu, "[REDACTED]")
    .slice(0, 1000);
}

export function sanitizeBackgroundAgentError(
  error: FetchBaseQueryError,
  token?: string | null,
): FetchBaseQueryError {
  const text = backgroundAgentErrorText(error, token);
  if (typeof error.status === "number")
    return { status: error.status, data: { detail: text } };
  if (error.status === "PARSING_ERROR")
    return {
      status: error.status,
      originalStatus: error.originalStatus,
      data: text,
      error: "Invalid response from background agents",
    };
  return { status: error.status, error: text };
}
