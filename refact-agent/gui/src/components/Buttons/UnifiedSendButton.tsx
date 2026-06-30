import React from "react";
import { Flex, Badge } from "@radix-ui/themes";
import { Clock, RefreshCcw, Send, Square, Zap } from "lucide-react";
import { Icon, IconButton, Tooltip } from "../ui";

type UnifiedSendButtonProps = {
  disabled?: boolean;
  isStreaming?: boolean;
  hasText: boolean;
  hasMessages: boolean;
  queuedCount?: number;
  onSend: () => void;
  onSendImmediately: () => void;
  onStop: () => void;
  onResend: () => void;
};

const QueuedBadge: React.FC<{ queuedCount: number }> = ({ queuedCount }) => {
  if (queuedCount <= 0) return null;

  return (
    <Badge
      color="amber"
      size="1"
      variant="soft"
      title={`${queuedCount} message(s) queued`}
    >
      <Icon icon={Clock} size="sm" />
      {queuedCount}
    </Badge>
  );
};

export const UnifiedSendButton: React.FC<UnifiedSendButtonProps> = ({
  disabled,
  isStreaming,
  hasText,
  hasMessages,
  queuedCount = 0,
  onSend,
  onSendImmediately,
  onStop,
  onResend,
}) => {
  if (isStreaming) {
    if (hasText) {
      return (
        <Flex align="center" gap="2">
          <QueuedBadge queuedCount={queuedCount} />
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                aria-label="Stop generation"
                icon={Square}
                onClick={(e) => {
                  e.preventDefault();
                  onStop();
                }}
                size="sm"
                variant="danger"
              />
            </Tooltip.Trigger>
            <Tooltip.Content side="top">Stop generation</Tooltip.Content>
          </Tooltip>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                aria-label="Send immediately"
                disabled={disabled}
                icon={Zap}
                onClick={(e) => {
                  e.preventDefault();
                  onSendImmediately();
                }}
                size="sm"
                variant="primary"
              />
            </Tooltip.Trigger>
            <Tooltip.Content side="top">
              Send immediately (next turn)
            </Tooltip.Content>
          </Tooltip>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                aria-label="Queue message"
                disabled={disabled}
                icon={Clock}
                onClick={(e) => {
                  e.preventDefault();
                  onSend();
                }}
                size="sm"
                variant="soft"
              />
            </Tooltip.Trigger>
            <Tooltip.Content side="top">
              Queue message (after tools complete)
            </Tooltip.Content>
          </Tooltip>
        </Flex>
      );
    }

    return (
      <Flex align="center" gap="2">
        <QueuedBadge queuedCount={queuedCount} />
        <Tooltip>
          <Tooltip.Trigger asChild>
            <IconButton
              aria-label="Stop generation"
              icon={Square}
              onClick={(e) => {
                e.preventDefault();
                onStop();
              }}
              size="sm"
              variant="danger"
            />
          </Tooltip.Trigger>
          <Tooltip.Content side="top">Stop generation</Tooltip.Content>
        </Tooltip>
      </Flex>
    );
  }

  if (!hasText && hasMessages) {
    return (
      <Flex align="center" gap="2">
        <QueuedBadge queuedCount={queuedCount} />
        <Tooltip>
          <Tooltip.Trigger asChild>
            <IconButton
              aria-label="Resend last messages"
              disabled={disabled}
              icon={RefreshCcw}
              onClick={(e) => {
                e.preventDefault();
                onResend();
              }}
              size="sm"
              variant="ghost"
            />
          </Tooltip.Trigger>
          <Tooltip.Content side="top">Resend last messages</Tooltip.Content>
        </Tooltip>
      </Flex>
    );
  }

  return (
    <Flex align="center" gap="2">
      <QueuedBadge queuedCount={queuedCount} />
      <Tooltip>
        <Tooltip.Trigger asChild>
          <IconButton
            aria-label="Send message"
            disabled={disabled}
            icon={Send}
            onClick={(e) => {
              e.preventDefault();
              onSend();
            }}
            size="sm"
            variant="primary"
          />
        </Tooltip.Trigger>
        <Tooltip.Content side="top">Send message</Tooltip.Content>
      </Tooltip>
    </Flex>
  );
};

export default UnifiedSendButton;
