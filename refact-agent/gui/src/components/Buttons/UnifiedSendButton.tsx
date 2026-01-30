import React from "react";
import { Flex, Badge, HoverCard, Text, IconButton } from "@radix-ui/themes";
import {
  PaperPlaneIcon,
  ClockIcon,
  LightningBoltIcon,
  StopIcon,
} from "@radix-ui/react-icons";
import styles from "./UnifiedSendButton.module.css";

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

const ResendIcon: React.FC = () => {
  return (
    <svg
      height="15"
      width="15"
      viewBox="0 0 24 24"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path
        fill="currentColor"
        fillRule="evenodd"
        clipRule="evenodd"
        d="M5.41421 3.58579C5.03914 3.21071 4.53043 3 4 3C2.89543 3 2 3.89543 2 5V10C2 10.5304 2.21071 11.0391 2.58579 11.4142C2.96086 11.7893 3.46957 12 4 12H9C10.1046 12 11 11.1046 11 10C11 8.89543 10.1046 8 9 8H6.41L7.35 7.06C8.02552 6.38448 8.82862 5.85207 9.70935 5.49295C10.5901 5.13383 11.5353 4.95467 12.49 4.96627C13.4447 4.97787 14.3855 5.17997 15.2566 5.56077C16.1278 5.94156 16.9141 6.49389 17.5709 7.18649C18.2277 7.87909 18.7424 8.69779 19.0849 9.59423C19.4275 10.4907 19.5912 11.447 19.5665 12.4091C19.5419 13.3713 19.3293 14.318 18.9415 15.1927C18.5538 16.0674 17.999 16.8533 17.31 17.5C16.9188 17.8828 16.9099 18.5159 17.2927 18.9071C17.6755 19.2983 18.3087 19.3072 18.6999 18.9244C19.6065 18.0376 20.3352 16.9812 20.8443 15.8149C21.3534 14.6485 21.633 13.3949 21.6667 12.1205C21.7003 10.8461 21.4873 9.5788 21.0399 8.38764C20.5925 7.19649 19.9193 6.10512 19.0587 5.17523C18.1982 4.24535 17.1661 3.49549 16.0219 2.96756C14.8777 2.43962 13.6439 2.14388 12.3877 2.09774C11.1316 2.0516 9.8797 2.25589 8.70273 2.69896C7.52577 3.14203 6.44699 3.81539 5.53 4.68L5.41421 3.58579ZM14.7073 12.7071C15.0978 12.3166 15.0978 11.6834 14.7073 11.2929C14.3168 10.9024 13.6836 10.9024 13.2931 11.2929L11.2931 13.2929C10.9026 13.6834 10.9026 14.3166 11.2931 14.7071L13.2931 16.7071C13.6836 17.0976 14.3168 17.0976 14.7073 16.7071C15.0978 16.3166 15.0978 15.6834 14.7073 15.2929L14.4142 15H17C17.5523 15 18 14.5523 18 14C18 13.4477 17.5523 13 17 13H14.4142L14.7073 12.7071Z"
      />
    </svg>
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
                variant="soft"
                color="red"
                size="1"
                type="button"
                onClick={(e) => {
                  e.preventDefault();
                  onStop();
                }}
              >
                <StopIcon />
              </IconButton>
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
                variant="soft"
                color="blue"
                disabled={disabled}
                size="1"
                type="button"
                onClick={(e) => {
                  e.preventDefault();
                  onSendImmediately();
                }}
                className={styles.sendButton}
              >
                <LightningBoltIcon />
              </IconButton>
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
                variant="soft"
                color="green"
                disabled={disabled}
                size="1"
                type="submit"
                onClick={(e) => {
                  e.preventDefault();
                  onSend();
                }}
                className={styles.sendButton}
              >
                <ClockIcon />
              </IconButton>
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
              variant="soft"
              color="red"
              size="1"
              type="button"
              onClick={(e) => {
                e.preventDefault();
                onStop();
              }}
            >
              <StopIcon />
            </IconButton>
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
              size="1"
              type="button"
              onClick={(e) => {
                e.preventDefault();
                onResend();
              }}
            >
              <ResendIcon />
            </IconButton>
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
};

export default UnifiedSendButton;
