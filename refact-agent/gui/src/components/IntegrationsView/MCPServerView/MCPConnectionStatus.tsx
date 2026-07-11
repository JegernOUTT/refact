import React from "react";
import { Badge, Button, Flex, Spinner, Text } from "../../ui";
import styles from "./MCPServerView.module.css";

type ConnectionStatusValue = string | Record<string, unknown>;

type MCPConnectionStatusProps = {
  status: ConnectionStatusValue;
  onReconnect: () => void;
  isReconnecting: boolean;
};

function getStatusLabel(status: ConnectionStatusValue): string {
  if (typeof status === "string") return status;
  if ("status" in status && typeof status.status === "string")
    return status.status;
  return "unknown";
}

function getStatusTone(
  label: string,
): React.ComponentProps<typeof Badge>["tone"] {
  const lower = label.toLowerCase();
  if (lower === "connected") return "success";
  if (lower === "connecting" || lower === "reconnecting") return "warning";
  if (lower === "error" || lower === "failed" || lower === "needs_auth")
    return "danger";
  if (lower === "disconnected") return "danger";
  return "muted";
}

function isSpinnerVisible(label: string, isReconnecting: boolean): boolean {
  const lower = label.toLowerCase();
  return isReconnecting || lower === "connecting" || lower === "reconnecting";
}

function getAttemptInfo(status: ConnectionStatusValue): string | null {
  if (typeof status !== "object") return null;
  const attempt =
    "attempt" in status && typeof status.attempt === "number"
      ? status.attempt
      : null;
  const maxAttempts =
    "max_attempts" in status && typeof status.max_attempts === "number"
      ? status.max_attempts
      : null;
  if (attempt !== null && maxAttempts !== null)
    return `Attempt ${attempt}/${maxAttempts}`;
  return null;
}

function getNextRetryInfo(status: ConnectionStatusValue): string | null {
  if (typeof status !== "object") return null;
  if (
    "next_retry_seconds" in status &&
    typeof status.next_retry_seconds === "number"
  ) {
    return `Next retry in ${status.next_retry_seconds}s`;
  }
  return null;
}

export const MCPConnectionStatus: React.FC<MCPConnectionStatusProps> = ({
  status,
  onReconnect,
  isReconnecting,
}) => {
  const label = getStatusLabel(status);
  const tone = getStatusTone(label);
  const showSpinner = isSpinnerVisible(label, isReconnecting);
  const attemptInfo = getAttemptInfo(status);
  const nextRetryInfo = getNextRetryInfo(status);

  return (
    <Flex
      align="center"
      className={styles.connectionStatus}
      gap="3"
      wrap="wrap"
    >
      <Flex align="center" gap="2">
        <Badge aria-label={`MCP connection status: ${label}`} tone={tone}>
          {label}
        </Badge>
        {showSpinner && <Spinner size="sm" label="Reconnecting" />}
      </Flex>
      {attemptInfo && (
        <Text size="1" color="gray">
          {attemptInfo}
        </Text>
      )}
      {nextRetryInfo && (
        <Text size="1" color="gray">
          {nextRetryInfo}
        </Text>
      )}
      <Button
        size="sm"
        variant="soft"
        onClick={onReconnect}
        disabled={isReconnecting}
      >
        {isReconnecting ? "Reconnecting..." : "Reconnect"}
      </Button>
      {typeof status === "object" &&
        "error" in status &&
        typeof status.error === "string" && (
          <Text as="p" size="1" color="red" className={styles.statusError}>
            {status.error}
          </Text>
        )}
      {typeof status === "object" &&
        "message" in status &&
        typeof status.message === "string" && (
          <Text as="p" size="1" color="red" className={styles.statusError}>
            {status.message}
          </Text>
        )}
    </Flex>
  );
};
