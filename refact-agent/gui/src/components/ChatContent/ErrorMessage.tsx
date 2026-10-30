import React from "react";
import { AlertTriangle } from "lucide-react";
import { Button, Icon } from "../ui";
import { humanizeIdentifier } from "../../utils/displayNames";
import { TranscriptItem, type TranscriptFact } from "./TranscriptItem";
import styles from "./ErrorMessage.module.css";
import type {
  ErrorMessage,
  RetryStatus,
  UserErrorCategory,
  UserErrorInfo,
} from "../../services/refact/types";

export type ErrorMessageCardProps = {
  errors: ErrorMessage[];
  onRetryGeneration?: () => void;
};

type ParsedError = {
  message: string;
  info?: UserErrorInfo;
  retry?: RetryStatus;
};

const ACTION_LABELS: Partial<Record<string, string>> = {
  retry: "Retry",
  compact: "Compact chat",
  check_auth: "Check auth",
  switch_model: "Switch model",
  check_billing: "Check billing",
  none: "Review error",
};

function isUserErrorCategory(value: unknown): value is UserErrorCategory {
  return (
    value === "ProviderTransient" ||
    value === "ProviderRateLimit" ||
    value === "ContextTooLarge" ||
    value === "AuthenticationFailed" ||
    value === "ModelUnavailable" ||
    value === "BillingQuota" ||
    value === "InvalidRequest" ||
    value === "NetworkFailure" ||
    value === "StreamCorrupted" ||
    value === "ToolSchemaInvalid" ||
    value === "ContentPolicy" ||
    value === "Unknown"
  );
}

function isUserErrorInfo(value: unknown): value is UserErrorInfo {
  if (!value || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  return (
    isUserErrorCategory(record.category) &&
    typeof record.title === "string" &&
    typeof record.explanation === "string" &&
    typeof record.suggested_action === "string" &&
    typeof record.is_retryable === "boolean"
  );
}

function isRetryStatus(value: unknown): value is RetryStatus {
  if (!value || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.attempt === "number" &&
    typeof record.max_attempts === "number" &&
    typeof record.delay_secs === "number" &&
    typeof record.in_progress === "boolean"
  );
}

function pickRetryStatus(error: ErrorMessage): RetryStatus | undefined {
  if (error.retry_status && isRetryStatus(error.retry_status)) {
    return error.retry_status;
  }
  if (isRetryStatus(error.extra?.retry_status)) {
    return error.extra.retry_status;
  }
  return undefined;
}

function parseStructuredError(error: ErrorMessage): ParsedError {
  const retry = pickRetryStatus(error);
  if (error.error_info) {
    return { message: error.content, info: error.error_info, retry };
  }
  if (isUserErrorInfo(error.extra?.error_info)) {
    return { message: error.content, info: error.extra.error_info, retry };
  }

  try {
    const parsed = JSON.parse(error.content) as unknown;
    if (!parsed || typeof parsed !== "object")
      return { message: error.content, retry };
    const record = parsed as Record<string, unknown>;
    const nested = record.error;
    if (nested && typeof nested === "object") {
      const nestedRecord = nested as Record<string, unknown>;
      if (isUserErrorInfo(nestedRecord.error_info)) {
        return {
          message:
            typeof nestedRecord.message === "string"
              ? nestedRecord.message
              : nestedRecord.error_info.raw_error ?? error.content,
          info: nestedRecord.error_info,
          retry,
        };
      }
    }
    if (isUserErrorInfo(record.error_info)) {
      return {
        message:
          typeof record.message === "string"
            ? record.message
            : record.error_info.raw_error ?? error.content,
        info: record.error_info,
        retry,
      };
    }
  } catch {
    return { message: error.content, retry };
  }
  return { message: error.content, retry };
}

function errorActionLabel(action: string): string {
  return ACTION_LABELS[action] ?? ACTION_LABELS.none ?? "Review error";
}

function shouldShowRawError(rawError: string, error: ParsedError): boolean {
  if (!rawError.trim()) return false;
  if (rawError === error.message) return !error.info;
  if (rawError === error.info?.explanation) return false;
  return true;
}

const HTTP_CODE = /\b([45]\d{2})\b/;
const PROVIDER_MODEL = /\b([a-z0-9_-]+\/[A-Za-z0-9._:-]+)\b/;
const MAX_TARGET_LENGTH = 80;

function firstLine(text: string): string {
  const line = text.split("\n").find((part) => part.trim().length > 0) ?? "";
  const trimmed = line.trim();
  return trimmed.length > MAX_TARGET_LENGTH
    ? `${trimmed.slice(0, MAX_TARGET_LENGTH - 1)}…`
    : trimmed;
}

function errorText(error: ParsedError): string {
  return [error.info?.raw_error, error.message].filter(Boolean).join("\n");
}

function errorTarget(errors: ParsedError[]): string {
  for (const error of errors) {
    const match = PROVIDER_MODEL.exec(errorText(error));
    if (match) return match[1];
  }
  const info = errors.find((error) => error.info)?.info;
  if (info) return info.title;
  return errors.length > 1
    ? `${errors.length} errors`
    : firstLine(errors[0]?.message ?? "");
}

function errorMeta(errors: ParsedError[]): string | null {
  const code = errors
    .map((error) => HTTP_CODE.exec(errorText(error))?.[1])
    .find((value): value is string => value !== undefined);
  const info = errors.find((error) => error.info)?.info;
  const reason = info ? humanizeIdentifier(info.category) : null;
  const parts = [code, reason].filter((part): part is string => Boolean(part));
  return parts.length > 0 ? parts.join(" · ") : null;
}

function retryHint(info: UserErrorInfo, retry: RetryStatus | undefined) {
  if (retry) {
    return `Auto-retrying in ${retry.delay_secs}s (attempt ${retry.attempt}/${retry.max_attempts}).`;
  }
  return info.is_retryable
    ? "Retrying may succeed after the condition clears."
    : "Retrying unchanged is unlikely to fix this.";
}

const ErrorCallout: React.FC<{ error: ParsedError; showTitle: boolean }> = ({
  error,
  showTitle,
}) => {
  const info = error.info;
  if (!info) {
    return <div className={styles.message}>{error.message}</div>;
  }
  const rawError = info.raw_error ?? error.message;
  return (
    <div className={styles.callout}>
      {showTitle && <strong>{info.title}</strong>}
      <div className={styles.message}>{info.explanation}</div>
      <div className={styles.hint}>{retryHint(info, undefined)}</div>
      {shouldShowRawError(rawError, error) && (
        <div className={styles.raw}>{rawError}</div>
      )}
    </div>
  );
};

export const ErrorMessageCard: React.FC<ErrorMessageCardProps> = ({
  errors,
  onRetryGeneration,
}) => {
  const parsedErrors = errors.map(parseStructuredError);
  const firstClassified = parsedErrors.find((error) => error.info)?.info;
  const canRetry = !!firstClassified && typeof onRetryGeneration === "function";
  const suggestion =
    firstClassified && firstClassified.suggested_action !== "retry"
      ? errorActionLabel(firstClassified.suggested_action)
      : null;
  const facts: TranscriptFact[] = suggestion
    ? [{ label: "Suggested", value: suggestion }]
    : [];

  return (
    <TranscriptItem
      data-testid="error-message-card"
      icon={<Icon icon={AlertTriangle} size="sm" />}
      verb={firstClassified ? "Request failed" : "Error"}
      target={errorTarget(parsedErrors)}
      meta={errorMeta(parsedErrors)}
      status="failed"
      defaultOpen
      summary={facts}
      error={
        <div className={styles.errors}>
          {parsedErrors.map((error, index) => (
            <ErrorCallout
              key={`${index}-${error.message}-${error.info?.category ?? "raw"}`}
              error={error}
              showTitle={parsedErrors.length > 1}
            />
          ))}
        </div>
      }
      actions={
        canRetry ? (
          <Button size="sm" variant="soft" onClick={onRetryGeneration}>
            Retry
          </Button>
        ) : null
      }
    />
  );
};
