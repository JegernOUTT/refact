import React from "react";
import { Badge, Button, Flex, Text } from "@radix-ui/themes";

type ConnectionStatusValue = string | Record<string, unknown>;

type MCPConnectionStatusProps = {
  status: ConnectionStatusValue;
  onReconnect: () => void;
  isReconnecting: boolean;
};

function getStatusLabel(status: ConnectionStatusValue): string {
  if (typeof status === "string") return status;
  if ("status" in status && typeof status.status === "string") return status.status;
  return "unknown";
}

function getStatusColor(label: string): "green" | "yellow" | "red" | "gray" {
  const lower = label.toLowerCase();
  if (lower === "connected") return "green";
  if (lower === "connecting" || lower === "reconnecting") return "yellow";
  if (lower === "error") return "red";
  return "gray";
}

export const MCPConnectionStatus: React.FC<MCPConnectionStatusProps> = ({
  status,
  onReconnect,
  isReconnecting,
}) => {
  const label = getStatusLabel(status);
  const color = getStatusColor(label);

  return (
    <Flex align="center" gap="3" wrap="wrap">
      <Badge color={color} radius="full" size="2">
        {label}
      </Badge>
      <Button
        size="1"
        variant="soft"
        onClick={onReconnect}
        disabled={isReconnecting}
      >
        {isReconnecting ? "Reconnecting..." : "Reconnect"}
      </Button>
      {typeof status === "object" && "error" in status && typeof status.error === "string" && (
        <Text size="1" color="red">
          {status.error}
        </Text>
      )}
    </Flex>
  );
};
