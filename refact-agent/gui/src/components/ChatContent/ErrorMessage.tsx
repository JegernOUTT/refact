import React from "react";
import { Badge, Box, Button, Card, Flex, Text } from "@radix-ui/themes";
import { ExclamationTriangleIcon } from "@radix-ui/react-icons";
import { Markdown } from "../Markdown";
import styles from "./ChatContent.module.css";
import type {
  ErrorMessage,
  UserErrorCategory,
  UserErrorInfo,
} from "../../services/refact/types";

export type ErrorMessageCardProps = {
  errors: ErrorMessage[];
};

type ParsedError = {
  message: string;
  info?: UserErrorInfo;
};

const CATEGORY_COLORS: Record<
  UserErrorCategory,
  React.ComponentProps<typeof Text>["color"]
> = {
  ProviderTransient: "amber",
  ProviderRateLimit: "amber",
  ContextTooLarge: "orange",
  AuthenticationFailed: "red",
  ModelUnavailable: "purple",
  BillingQuota: "red",
  InvalidRequest: "red",
  NetworkFailure: "amber",
  StreamCorrupted: "amber",
  ToolSchemaInvalid: "red",
  ContentPolicy: "red",
  Unknown: "red",
};

const ACTION_LABELS: Record<string, string> = {
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

function parseStructuredError(error: ErrorMessage): ParsedError {
  if (error.error_info) {
    return { message: error.content, info: error.error_info };
  }
  if (isUserErrorInfo(error.extra?.error_info)) {
    return { message: error.content, info: error.extra.error_info };
  }

  try {
    const parsed = JSON.parse(error.content) as unknown;
    if (!parsed || typeof parsed !== "object") return { message: error.content };
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
      };
    }
  } catch {
    return { message: error.content };
  }
  return { message: error.content };
}

function errorActionLabel(action: string): string {
  return ACTION_LABELS[action] ?? ACTION_LABELS.none;
}

const ClassifiedError: React.FC<{ error: ParsedError }> = ({ error }) => {
  const info = error.info;
  if (!info) {
    return (
      <Box className={styles.errorMessageBody}>
        <Markdown>{error.message}</Markdown>
      </Box>
    );
  }

  const color = CATEGORY_COLORS[info.category];
  const rawError = info.raw_error ?? error.message;

  return (
    <Box className={styles.errorMessageBody}>
      <Flex direction="column" gap="2">
        <Flex align="center" justify="between" gap="2" wrap="wrap">
          <Flex align="center" gap="2" wrap="wrap">
            <Text size="2" weight="bold" color={color}>
              {info.title}
            </Text>
            <Badge color={color} variant="soft">
              {info.category}
            </Badge>
          </Flex>
          <Button size="1" variant="soft" color={color}>
            {errorActionLabel(info.suggested_action)}
          </Button>
        </Flex>
        <Text size="2">{info.explanation}</Text>
        <Text size="1" color="gray">
          {info.is_retryable
            ? "Retrying may succeed after the condition clears."
            : "Retrying unchanged is unlikely to fix this."}
        </Text>
        {rawError && (
          <Box className={styles.errorMessageRaw}>
            <Markdown>{rawError}</Markdown>
          </Box>
        )}
      </Flex>
    </Box>
  );
};

export const ErrorMessageCard: React.FC<ErrorMessageCardProps> = ({
  errors,
}) => {
  const parsedErrors = errors.map(parseStructuredError);
  const firstClassified = parsedErrors.find((error) => error.info)?.info;
  const title = firstClassified
    ? firstClassified.title
    : errors.length === 1
      ? "Generation error"
      : `${errors.length} generation errors`;
  const color = firstClassified
    ? CATEGORY_COLORS[firstClassified.category]
    : "red";

  return (
    <Card className={styles.errorMessageCard} variant="surface">
      <Flex direction="column" gap="2">
        <Flex align="center" gap="2">
          <Box className={styles.errorMessageIcon}>
            <ExclamationTriangleIcon width="15" height="15" />
          </Box>
          <Text size="2" weight="medium" color={color}>
            {title}
          </Text>
        </Flex>
        <Flex direction="column" gap="2">
          {parsedErrors.map((error, index) => (
            <ClassifiedError
              key={`${index}-${error.message}-${error.info?.category ?? "raw"}`}
              error={error}
            />
          ))}
        </Flex>
      </Flex>
    </Card>
  );
};
