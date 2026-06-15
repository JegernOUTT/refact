import React from "react";
import { Flex, Badge, HoverCard, Text } from "@radix-ui/themes";
import { Clock, RefreshCcw, Send, Square, Zap } from "lucide-react";
import { Icon, IconButton } from "../ui";

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
          <HoverCard.Root>
            <HoverCard.Trigger>
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
            </HoverCard.Trigger>
            <HoverCard.Content size="1" side="top">
              <Text as="p" size="2">
                Stop generation
              </Text>
            </HoverCard.Content>
          </HoverCard.Root>
          <HoverCard.Root>
            <HoverCard.Trigger>
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
            </HoverCard.Trigger>
            <HoverCard.Content size="1" side="top">
              <Text as="p" size="2">
                Send immediately (next turn)
              </Text>
            </HoverCard.Content>
          </HoverCard.Root>
          <HoverCard.Root>
            <HoverCard.Trigger>
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
            </HoverCard.Trigger>
            <HoverCard.Content size="1" side="top">
              <Text as="p" size="2">
                Queue message (after tools complete)
              </Text>
            </HoverCard.Content>
          </HoverCard.Root>
        </Flex>
      );
    }

    return (
      <Flex align="center" gap="2">
        <QueuedBadge queuedCount={queuedCount} />
        <HoverCard.Root>
          <HoverCard.Trigger>
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
          </HoverCard.Trigger>
          <HoverCard.Content size="1" side="top">
            <Text as="p" size="2">
              Stop generation
            </Text>
          </HoverCard.Content>
        </HoverCard.Root>
      </Flex>
    );
  }

  if (!hasText && hasMessages) {
    return (
      <Flex align="center" gap="2">
        <QueuedBadge queuedCount={queuedCount} />
        <HoverCard.Root>
          <HoverCard.Trigger>
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
          </HoverCard.Trigger>
          <HoverCard.Content size="1" side="top">
            <Text as="p" size="2">
              Resend last messages
            </Text>
          </HoverCard.Content>
        </HoverCard.Root>
      </Flex>
    );
  }

  return (
    <Flex align="center" gap="2">
      <QueuedBadge queuedCount={queuedCount} />
      <HoverCard.Root>
        <HoverCard.Trigger>
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
        </HoverCard.Trigger>
        <HoverCard.Content size="1" side="top">
          <Text as="p" size="2">
            Send message
          </Text>
        </HoverCard.Content>
      </HoverCard.Root>
    </Flex>
  );
};

export default UnifiedSendButton;
