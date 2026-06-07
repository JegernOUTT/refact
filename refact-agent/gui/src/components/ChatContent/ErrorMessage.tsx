import React from "react";
import { Flex, Text } from "@radix-ui/themes";
import { AlertTriangle, LoaderCircle } from "lucide-react";
import { Badge, Button, Icon } from "../ui";
import styles from "./ChatContent.module.css";
import type {
  ErrorMessage,
  RetryStatus,
  UserErrorCategory,
  UserErrorInfo,
} from "../../services/refact/types";

export type ErrorMessageCardProps = {
  errors: ErrorMessage[];
};

type ParsedError = {
  message: string;
  info?: UserErrorInfo;
  retry?: RetryStatus;
};

type ErrorTone = React.ComponentProps<typeof Badge>["tone"];

const CATEGORY_TONES: Record<UserErrorCategory, ErrorTone> = {
  ProviderTransient: "warning",
  ProviderRateLimit: "warning",
  ContextTooLarge: "warning",
  AuthenticationFailed: "danger",
  ModelUnavailable: "accent",
  BillingQuota: "danger",
  InvalidRequest: "danger",
  NetworkFailure: "warning",
  StreamCorrupted: "warning",
  ToolSchemaInvalid: "danger",
  ContentPolicy: "danger",
  Unknown: "danger",
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

const RetryingBadge: React.FC<{
  retry: RetryStatus;
  tone: ErrorTone;
}> = ({ retry, tone }) => (
  <Badge tone={tone}>
    <Icon
      className={styles.retryingBadgeIcon}
      icon={LoaderCircle}
      size="sm"
      tone={tone === "danger" ? "danger" : "warning"}
    />
    Retrying {retry.delay_secs}s · {retry.attempt}/{retry.max_attempts}
  </Badge>
);

const ClassifiedError: React.FC<{
  error: ParsedError;
  showHeader: boolean;
}> = ({ error, showHeader }) => {
  const info = error.info;
  if (!info) {
    return <div className={styles.errorMessageBody}>{error.message}</div>;
  }

  const tone = CATEGORY_TONES[info.category];
  const rawError = info.raw_error ?? error.message;
  const retry = error.retry?.in_progress ? error.retry : undefined;

  return (
    <Flex direction="column" gap="2" className={styles.errorMessageBody}>
      {showHeader && (
        <Flex align="center" justify="between" gap="2" wrap="wrap">
          <Flex align="center" gap="2" wrap="wrap">
            <Text size="2" weight="bold">
              {info.title}
            </Text>
            <Badge tone={tone}>{info.category}</Badge>
          </Flex>
          {retry ? (
            <RetryingBadge retry={retry} tone={tone} />
          ) : (
            <Button size="sm" variant={tone === "danger" ? "danger" : "soft"}>
              {errorActionLabel(info.suggested_action)}
            </Button>
          )}
        </Flex>
      )}
      <Text size="2">{info.explanation}</Text>
      <Text className={styles.errorMessageHint} size="1">
        {retry
          ? `Auto-retrying in ${retry.delay_secs}s (attempt ${retry.attempt}/${retry.max_attempts}).`
          : info.is_retryable
            ? "Retrying may succeed after the condition clears."
            : "Retrying unchanged is unlikely to fix this."}
      </Text>
      {shouldShowRawError(rawError, error) && (
        <div className={styles.errorMessageRaw}>{rawError}</div>
      )}
    </Flex>
  );
};

export const ErrorMessageCard: React.FC<ErrorMessageCardProps> = ({
  errors,
}) => {
  const parsedErrors = errors.map(parseStructuredError);
  const firstClassified = parsedErrors.find((error) => error.info)?.info;
  const latestRetry = parsedErrors
    .map((error) => error.retry)
    .filter((retry): retry is RetryStatus => Boolean(retry?.in_progress))
    .pop();
  const title = firstClassified
    ? firstClassified.title
    : errors.length === 1
      ? "Generation error"
      : `${errors.length} generation errors`;
  const tone = firstClassified ? CATEGORY_TONES[firstClassified.category] : "danger";
  const showPerErrorHeader = parsedErrors.length > 1;

  return (
    <div className={`${styles.errorMessageCard} rf-enter-rise`}>
      <Flex direction="column" gap="2">
        <Flex align="center" justify="between" gap="2" wrap="wrap">
          <Flex align="center" gap="2" wrap="wrap">
            <span className={styles.errorMessageIcon}>
              <Icon icon={AlertTriangle} size="md" tone="danger" />
            </span>
            <Text size="2" weight="medium">
              {title}
            </Text>
            {firstClassified && <Badge tone={tone}>{firstClassified.category}</Badge>}
          </Flex>
          {firstClassified &&
            !showPerErrorHeader &&
            (latestRetry ? (
              <RetryingBadge retry={latestRetry} tone={tone} />
            ) : (
              <Button size="sm" variant={tone === "danger" ? "danger" : "soft"}>
                {errorActionLabel(firstClassified.suggested_action)}
              </Button>
            ))}
        </Flex>
        <Flex direction="column" gap="3">
          {parsedErrors.map((error, index) => (
            <ClassifiedError
              key={`${index}-${error.message}-${error.info?.category ?? "raw"}`}
              error={error}
              showHeader={showPerErrorHeader}
            />
          ))}
        </Flex>
      </Flex>
    </div>
  );
};
