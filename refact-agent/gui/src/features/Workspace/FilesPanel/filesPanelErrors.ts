import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";

const PRIVACY_BLOCKED_PREFIX = "Blocked by privacy rules:";

export const errorStatus = (error: unknown): number | string | null => {
  const candidate = error as FetchBaseQueryError | undefined;
  return candidate?.status ?? null;
};

export const errorDetail = (error: unknown): string | null => {
  const data = (error as FetchBaseQueryError | undefined)?.data;
  if (typeof data !== "object" || data === null) return null;
  const detail = (data as { detail?: unknown }).detail;
  return typeof detail === "string" && detail.length > 0 ? detail : null;
};

export const isAccessDenied = (error: unknown): boolean =>
  errorStatus(error) === 403;

export const isPrivacyBlocked = (error: unknown): boolean =>
  isAccessDenied(error) &&
  (errorDetail(error)?.startsWith(PRIVACY_BLOCKED_PREFIX) ?? false);
