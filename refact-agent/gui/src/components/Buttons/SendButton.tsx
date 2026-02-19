import React from "react";
import {
  DropdownMenu,
  IconButton,
  Flex,
  Badge,
  HoverCard,
  Text,
} from "@radix-ui/themes";
import {
  PaperPlaneIcon,
  CaretDownIcon,
  ClockIcon,
  LightningBoltIcon,
} from "@radix-ui/react-icons";

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
            <ClockIcon width={12} height={12} />
            {queuedCount}
          </Badge>
        )}
        <HoverCard.Root>
          <HoverCard.Trigger>
            <IconButton
              variant="ghost"
              disabled={disabled}
              title={undefined}
              size="1"
              type="submit"
              onClick={(e) => {
                e.preventDefault();
                onSend();
              }}
            >
              <PaperPlaneIcon />
            </IconButton>
          </HoverCard.Trigger>
          <HoverCard.Content size="1" side="top">
            <Text as="p" size="2">
              Send message
            </Text>
          </HoverCard.Content>
        </HoverCard.Root>
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
          <ClockIcon width={12} height={12} />
          {queuedCount}
        </Badge>
      )}
      <HoverCard.Root>
        <HoverCard.Trigger>
          <DropdownMenu.Root>
            <DropdownMenu.Trigger>
              <IconButton
                variant="ghost"
                disabled={disabled}
                title={undefined}
                size="1"
              >
                <PaperPlaneIcon />
                <CaretDownIcon width={12} height={12} />
              </IconButton>
            </DropdownMenu.Trigger>

            <DropdownMenu.Content size="1" align="end">
              <DropdownMenu.Item onSelect={() => onSend()}>
                <ClockIcon width={14} height={14} />
                Queue message
              </DropdownMenu.Item>
              <DropdownMenu.Item onSelect={() => onSendImmediately()}>
                <LightningBoltIcon width={14} height={14} />
                Send next
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Root>
        </HoverCard.Trigger>
        <HoverCard.Content size="1" side="top">
          <Text as="p" size="2">
            Send options
          </Text>
        </HoverCard.Content>
      </HoverCard.Root>
    </Flex>
  );
};

export default SendButtonWithDropdown;
