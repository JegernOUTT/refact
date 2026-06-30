import React from "react";
import { Badge, DropdownMenu, Flex } from "@radix-ui/themes";
import { Clock, ChevronsUp, Send, Zap } from "lucide-react";
import { Icon, IconButton, Tooltip } from "../ui";

type SendButtonProps = {
  disabled?: boolean;
  isStreaming?: boolean;
  queuedCount?: number;
  onSend: () => void;
  onSendImmediately: () => void;
};

export const SendButtonWithDropdown: React.FC<SendButtonProps> = ({
  disabled,
  isStreaming,
  queuedCount = 0,
  onSend,
  onSendImmediately,
}) => {
  const showDropdown = isStreaming && !disabled;

  if (!showDropdown) {
    return (
      <Flex align="center" gap="2">
        {queuedCount > 0 && (
          <Badge
            color="amber"
            size="1"
            variant="soft"
            title={`${queuedCount} message(s) queued`}
          >
            <Icon icon={Clock} size="sm" />
            {queuedCount}
          </Badge>
        )}
        <Tooltip>
          <Tooltip.Trigger asChild>
            <IconButton
              aria-label="Send message"
              disabled={disabled}
              icon={Send}
              title={undefined}
              size="sm"
              type="submit"
              variant="ghost"
              onClick={(e) => {
                e.preventDefault();
                onSend();
              }}
            />
          </Tooltip.Trigger>
          <Tooltip.Content side="top">Send message</Tooltip.Content>
        </Tooltip>
      </Flex>
    );
  }

  return (
    <Flex align="center" gap="2">
      {queuedCount > 0 && (
        <Badge
          color="amber"
          size="1"
          variant="soft"
          title={`${queuedCount} message(s) queued`}
        >
          <Icon icon={Clock} size="sm" />
          {queuedCount}
        </Badge>
      )}
      <DropdownMenu.Root>
        <Tooltip>
          <Tooltip.Trigger asChild>
            <DropdownMenu.Trigger>
              <IconButton
                aria-label="Send options"
                disabled={disabled}
                icon={Send}
                title={undefined}
                size="sm"
                variant="ghost"
              />
            </DropdownMenu.Trigger>
          </Tooltip.Trigger>
          <Tooltip.Content side="top">Send options</Tooltip.Content>
        </Tooltip>

        <DropdownMenu.Content size="1" align="end">
          <DropdownMenu.Item onSelect={() => onSend()}>
            <Icon icon={Clock} size="sm" />
            Queue message
          </DropdownMenu.Item>
          <DropdownMenu.Item onSelect={() => onSendImmediately()}>
            <Icon icon={Zap} size="sm" />
            Send next
          </DropdownMenu.Item>
        </DropdownMenu.Content>
      </DropdownMenu.Root>
      <Icon icon={ChevronsUp} size="sm" />
    </Flex>
  );
};

export default SendButtonWithDropdown;
