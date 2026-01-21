import React, { useMemo } from "react";
import { Card, Flex, Text, Box, Spinner, Badge } from "@radix-ui/themes";
import {
  ChatBubbleIcon,
  DotFilledIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  PauseIcon,
  CrossCircledIcon,
} from "@radix-ui/react-icons";
import { CloseButton } from "../Buttons/Buttons";
import { IconButton } from "@radix-ui/themes";
import { OpenInNewWindowIcon } from "@radix-ui/react-icons";
import type { ChatHistoryItem } from "../../features/History/historySlice";
import {
  useChatSessionStates,
  SessionState,
} from "../../hooks/useStreamingChatIds";
import { getTotalCostMeteringForMessages } from "../../utils/getMetering";
import { Coin } from "../../images";

export const HistoryItem: React.FC<{
  historyItem: ChatHistoryItem;
  onClick: () => void;
  onDelete: (id: string) => void;
  onOpenInTab?: (id: string) => void;
  disabled: boolean;
  badge?: string;
  childCount?: number;
  isExpanded?: boolean;
  onToggleExpand?: () => void;
}> = ({
  historyItem,
  onClick,
  onDelete,
  onOpenInTab,
  disabled,
  badge,
  childCount,
  isExpanded,
  onToggleExpand,
}) => {
  const dateCreated = new Date(historyItem.createdAt);
  const dateTimeString = dateCreated.toLocaleString();
  const chatSessionStates = useChatSessionStates();

  const totalCost = useMemo(() => {
    const totals = getTotalCostMeteringForMessages(historyItem.messages);

    if (totals === null) return null;

    return (
      totals.metering_coins_cache_creation +
      totals.metering_coins_cache_read +
      totals.metering_coins_generated +
      totals.metering_coins_prompt
    );
  }, [historyItem.messages]);

  const getSessionState = (): SessionState | null => {
    return chatSessionStates[historyItem.id] as SessionState | null;
  };

  const sessionState = getSessionState();
  const isWorking =
    sessionState === "generating" || sessionState === "executing_tools";
  const isPaused = sessionState === "paused" || sessionState === "waiting_ide";
  const isError = sessionState === "error";
  return (
    <Box style={{ position: "relative", width: "100%" }}>
      <Card
        style={{
          width: "100%",
          opacity: disabled ? 0.8 : 1,
        }}
        variant="surface"
        className="rt-Button"
        asChild
        role="button"
      >
        <button
          disabled={disabled}
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            onClick();
          }}
        >
          <Flex gap="1" align="center">
            {isWorking && <Spinner style={{ minWidth: 16, minHeight: 16 }} />}
            {!isWorking && isPaused && (
              <PauseIcon
                style={{
                  minWidth: 16,
                  minHeight: 16,
                  color: "var(--yellow-9)",
                }}
              />
            )}
            {!isWorking && !isPaused && isError && (
              <CrossCircledIcon
                style={{ minWidth: 16, minHeight: 16, color: "var(--red-9)" }}
              />
            )}
            {!isWorking &&
              !isPaused &&
              !isError &&
              historyItem.read === false && (
                <DotFilledIcon style={{ minWidth: 16, minHeight: 16 }} />
              )}
            <Text
              as="div"
              size="2"
              weight="bold"
              style={{
                textOverflow: "ellipsis",
                overflow: "hidden",
                whiteSpace: "nowrap",
              }}
            >
              {historyItem.title}
            </Text>
            {badge && (
              <Badge size="1" color="gray" variant="soft">
                {badge}
              </Badge>
            )}
          </Flex>

          <Flex justify="between" mt="8px">
            <Flex gap="4">
              <Text
                size="1"
                style={{ display: "flex", gap: "4px", alignItems: "center" }}
              >
                <ChatBubbleIcon /> {historyItem.message_count ?? 0}
              </Text>
              {totalCost !== null && (
                <Text
                  size="1"
                  style={{ display: "flex", gap: "4px", alignItems: "center" }}
                >
                  <Coin width="15px" height="15px" /> {Math.round(totalCost)}
                </Text>
              )}
            </Flex>

            <Text size="1">{dateTimeString}</Text>
          </Flex>
        </button>
      </Card>

      {childCount !== undefined && onToggleExpand && (
        <Box
          onClick={(e) => {
            e.stopPropagation();
            onToggleExpand();
          }}
          style={{
            cursor: "pointer",
            padding: "4px 8px",
            borderRadius: "0 0 4px 4px",
            marginTop: "-2px",
            background: "var(--gray-a3)",
          }}
        >
          <Flex align="center" justify="center" gap="1">
            <Text size="1" color="gray">
              {childCount} related {childCount === 1 ? "chat" : "chats"}
            </Text>
            {isExpanded ? (
              <ChevronDownIcon width={12} height={12} />
            ) : (
              <ChevronRightIcon width={12} height={12} />
            )}
          </Flex>
        </Box>
      )}

      <Flex
        position="absolute"
        top="6px"
        right="6px"
        gap="1"
        justify="end"
        align="center"
      >
        {onOpenInTab && (
          <IconButton
            size="1"
            title="open in tab"
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              onOpenInTab(historyItem.id);
            }}
            variant="ghost"
          >
            <OpenInNewWindowIcon width="10" height="10" />
          </IconButton>
        )}

        <CloseButton
          size="1"
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            onDelete(historyItem.id);
          }}
          iconSize={10}
          title="delete chat"
        />
      </Flex>
    </Box>
  );
};
