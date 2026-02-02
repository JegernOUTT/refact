import React, { useCallback, useMemo, useEffect, useState } from "react";
import { v4 as uuidv4 } from "uuid";
import {
  ChatContextFile,
  ChatMessages,
  DiffChunk,
  DiffMessage,
  isChatContextFileMessage,
  isDiffMessage,
  isToolMessage,
  isSystemMessage,
  UserMessage,
} from "../../services/refact";
import { UserInput } from "./UserInput";
import { ScrollArea, ScrollAreaWithAnchor } from "../ScrollArea";
import { Flex, Container, Button, Box } from "@radix-ui/themes";
import styles from "./ChatContent.module.css";
import { ContextFiles } from "./ContextFiles";
import { SystemPrompt } from "./SystemPrompt";
import { AssistantInput } from "./AssistantInput";
import { PlainText } from "./PlainText";
import { useAppDispatch, useAppSelector, useDiffFileReload } from "../../hooks";
import {
  selectIntegration,
  selectIsStreamingById,
  selectIsWaitingById,
  selectMessagesById,
  selectQueuedItemsById,
  selectSnapshotReceivedById,
  selectThreadById,
  selectChatId,
  selectThreadPauseById,
} from "../../features/Chat/Thread/selectors";
import {
  createChatWithId,
  switchToThread,
} from "../../features/Chat/Thread/actions";
import { GroupedDiffs } from "./DiffContent";
import { popBackTo } from "../../features/Pages/pagesSlice";
import { ChatLinks, UncommittedChangesWarning } from "../ChatLinks";
import { PlaceHolderText } from "./PlaceHolderText";
import { QueuedMessage } from "./QueuedMessage";
import { selectSseConnectionForChat } from "../../features/Connection";
import { LogoAnimation } from "../LogoAnimation/LogoAnimation.tsx";
import { ChatLoading } from "./ChatLoading";
import {
  removeMessage,
  branchFromChat,
} from "../../services/refact/chatCommands";
import { selectLspPort, selectApiKey } from "../../features/Config/configSlice";

export type ChatContentProps = {
  onRetry: (index: number, question: UserMessage["content"]) => void;
  onStopStreaming: () => void;
};

export const ChatContent: React.FC<ChatContentProps> = ({
  onStopStreaming,
  onRetry,
}) => {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectChatId);
  const [renderChatId, setRenderChatId] = useState(chatId);

  useEffect(() => {
    if (chatId === renderChatId) return;
    const rafId = requestAnimationFrame(() => {
      setRenderChatId(chatId);
    });
    return () => cancelAnimationFrame(rafId);
  }, [chatId, renderChatId]);

  const switching = chatId !== renderChatId;

  const messages = useAppSelector((s) => selectMessagesById(s, renderChatId));
  const queuedItems = useAppSelector((s) =>
    selectQueuedItemsById(s, renderChatId),
  );
  const isStreaming = useAppSelector((s) =>
    selectIsStreamingById(s, renderChatId),
  );
  const snapshotReceived = useAppSelector((s) =>
    selectSnapshotReceivedById(s, renderChatId),
  );
  const thread = useAppSelector((s) => selectThreadById(s, renderChatId));
  const sseConnection = useAppSelector((s) =>
    selectSseConnectionForChat(s, renderChatId),
  );
  const sseStatus = sseConnection?.status ?? null;

  const isConfig = thread !== null && thread.mode === "CONFIGURE";
  const isWaiting = useAppSelector((s) => selectIsWaitingById(s, renderChatId));
  const integrationMeta = useAppSelector(selectIntegration);
  const isWaitingForConfirmation = useAppSelector((s) =>
    selectThreadPauseById(s, renderChatId),
  );
  const lspPort = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);

  const handleBranch = useCallback(
    (messageId: string) => {
      const newChatId = uuidv4();
      const title = `[branched] ${thread?.title ?? "Chat"}`;

      dispatch(
        createChatWithId({
          id: newChatId,
          title,
        }),
      );

      dispatch(switchToThread({ id: newChatId }));

      void branchFromChat(
        newChatId,
        renderChatId,
        messageId,
        lspPort,
        apiKey ?? undefined,
      ).catch((err) => {
        // eslint-disable-next-line no-console
        console.error("Failed to branch chat:", err);
      });
    },
    [dispatch, thread?.title, renderChatId, lspPort, apiKey],
  );

  const handleDelete = useCallback(
    (messageId: string) => {
      void removeMessage(
        renderChatId,
        messageId,
        lspPort,
        apiKey ?? undefined,
      ).catch((err) => {
        // eslint-disable-next-line no-console
        console.error("Failed to delete message:", err);
      });
    },
    [renderChatId, lspPort, apiKey],
  );

  const onRetryWrapper = useCallback(
    (index: number, question: UserMessage["content"]) => {
      onRetry(index, question);
    },
    [onRetry],
  );

  const handleReturnToConfigurationClick = useCallback(() => {
    onStopStreaming();
    dispatch(
      popBackTo({
        name: "integrations page",
        projectPath: thread?.integration?.project,
        integrationName: thread?.integration?.name,
        integrationPath: thread?.integration?.path,
        wasOpenedThroughChat: true,
      }),
    );
  }, [
    onStopStreaming,
    dispatch,
    thread?.integration?.project,
    thread?.integration?.name,
    thread?.integration?.path,
  ]);

  const shouldConfigButtonBeVisible = useMemo(() => {
    return isConfig && !integrationMeta?.path?.includes("project_summary");
  }, [isConfig, integrationMeta?.path]);

  useDiffFileReload();

  const showLoading =
    switching ||
    (!snapshotReceived && messages.length === 0) ||
    (sseStatus === "connecting" && messages.length === 0);

  return (
    <ScrollAreaWithAnchor.ScrollArea
      style={{ flexGrow: 1, height: "auto", position: "relative" }}
      scrollbars="vertical"
      type={isWaiting || isStreaming ? "auto" : "hover"}
      fullHeight
    >
      <Flex
        direction="column"
        className={styles.content}
        data-element="ChatContent"
        p="2"
        gap="1"
      >
        {showLoading && <ChatLoading />}
        {!showLoading && messages.length === 0 && (
          <Container>
            <PlaceHolderText />
          </Container>
        )}
        {!showLoading && messages.length > 0 && (
          <Container>
            {renderMessagesFast(
              messages,
              onRetryWrapper,
              handleBranch,
              handleDelete,
              isStreaming,
            )}
          </Container>
        )}
        <Container>
          <UncommittedChangesWarning />
        </Container>
        <Container pt="4" pb="8">
          {!isWaitingForConfirmation && (
            <LogoAnimation
              size="8"
              isStreaming={isStreaming}
              isWaiting={isWaiting}
            />
          )}
        </Container>
      </Flex>

      <Box
        style={{
          position: "absolute",
          bottom: 0,
          maxWidth: "100%",
        }}
      >
        <ScrollArea scrollbars="horizontal">
          <Flex align="start" gap="3" pb="2">
            {shouldConfigButtonBeVisible && (
              <Button
                color="gray"
                title="Return to configuration page"
                onClick={handleReturnToConfigurationClick}
              >
                Return
              </Button>
            )}

            <ChatLinks />
          </Flex>
        </ScrollArea>
      </Box>

      {queuedItems.length > 0 && (
        <Box className={styles.queuedMessagesContainer}>
          <Flex direction="column" gap="2" align="end">
            {queuedItems.map((item, index) => (
              <QueuedMessage
                key={item.client_request_id}
                queuedItem={item}
                position={index + 1}
              />
            ))}
          </Flex>
        </Box>
      )}
    </ScrollAreaWithAnchor.ScrollArea>
  );
};

ChatContent.displayName = "ChatContent";

function getMessageKey(message: ChatMessages[number], index: number): string {
  if (message.message_id) return message.message_id;
  if ("tool_call_id" in message && message.tool_call_id) {
    return `${message.role}-${message.tool_call_id}-${index}`;
  }
  return `${message.role}-${index}`;
}

function extractUserMessageText(content: UserMessage["content"]): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  for (const item of content) {
    if ("type" in item && item.type === "text" && "text" in item) {
      return item.text;
    }
    if ("m_type" in item && item.m_type === "text" && "m_content" in item) {
      return String(item.m_content);
    }
  }
  return "";
}

function computeHiddenQaMessageIndices(messages: ChatMessages): Set<number> {
  const hiddenIndices = new Set<number>();
  const askQuestionsToolIds = new Map<string, number>();

  for (let i = 0; i < messages.length; i++) {
    const msg = messages[i];
    if (msg.role === "assistant" && "tool_calls" in msg && msg.tool_calls) {
      for (const tc of msg.tool_calls) {
        if (tc.function.name === "ask_questions" && tc.id) {
          askQuestionsToolIds.set(tc.id, i);
        }
      }
    }
  }

  for (const [toolCallId, assistantIdx] of askQuestionsToolIds) {
    let foundToolResult = false;
    for (let j = assistantIdx + 1; j < messages.length; j++) {
      const msg = messages[j];
      if (isToolMessage(msg) && msg.tool_call_id === toolCallId) {
        foundToolResult = true;
        continue;
      }
      if (foundToolResult && msg.role === "user") {
        const contentStr = extractUserMessageText(msg.content);
        if (contentStr.startsWith(`[QA:${toolCallId}]`)) {
          hiddenIndices.add(j);
        }
        break;
      }
    }
  }

  return hiddenIndices;
}

function renderMessagesFast(
  messages: ChatMessages,
  onRetry: (index: number, question: UserMessage["content"]) => void,
  onBranch: (messageId: string) => void,
  onDelete: (messageId: string) => void,
  isStreaming: boolean,
): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  if (messages.length === 0) return nodes;

  const hiddenQaIndices = computeHiddenQaMessageIndices(messages);

  let lastUserIdx = -1;
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (msg.role === "user" && !hiddenQaIndices.has(i)) {
      lastUserIdx = i;
      break;
    }
  }

  for (let i = 0; i < messages.length; i++) {
    const head = messages[i];

    if (isToolMessage(head)) continue;

    if (head.role === "plain_text") {
      const key = getMessageKey(head, i);
      nodes.push(<PlainText key={key}>{head.content}</PlainText>);
      continue;
    }

    if (head.role === "assistant") {
      const key = getMessageKey(head, i);
      const contextFilesAfter: React.ReactNode[] = [];
      const diffMessagesAfter: DiffMessage[] = [];
      const contextFilesByToolId: Record<
        string,
        ChatContextFile[] | undefined
      > = {};
      const diffsByToolId: Record<string, DiffChunk[] | undefined> = {};

      const READ_TOOLS = new Set([
        "cat",
        "tree",
        "search_pattern",
        "search_semantic",
        "search_symbol_definition",
        "web",
        "web_search",
        "knowledge",
        "search_trajectories",
        "get_trajectory_context",
      ]);

      const EDIT_TOOLS = new Set([
        "create_textdoc",
        "update_textdoc",
        "replace_textdoc",
        "update_textdoc_regex",
        "update_textdoc_by_lines",
        "update_textdoc_anchored",
        "apply_patch",
        "undo_textdoc",
        "rm",
      ]);

      const toolCalls = head.tool_calls ?? [];
      const eligibleToolCalls = toolCalls.filter(
        (tc) => tc.id && tc.function.name && READ_TOOLS.has(tc.function.name),
      );
      const eligibleToolIds = new Set(
        eligibleToolCalls
          .map((tc) => tc.id)
          .filter((id): id is string => Boolean(id)),
      );
      const lastEligibleToolId =
        eligibleToolCalls.length > 0
          ? eligibleToolCalls[eligibleToolCalls.length - 1].id
          : null;

      const editToolCalls = toolCalls.filter(
        (tc) => tc.id && tc.function.name && EDIT_TOOLS.has(tc.function.name),
      );
      const editToolIds = new Set(
        editToolCalls
          .map((tc) => tc.id)
          .filter((id): id is string => Boolean(id)),
      );

      let j = i + 1;
      while (j < messages.length) {
        const nextMsg = messages[j];

        if (isToolMessage(nextMsg)) {
          j++;
          continue;
        }

        if (isChatContextFileMessage(nextMsg)) {
          if (
            nextMsg.tool_call_id === "knowledge_enrichment" ||
            nextMsg.tool_call_id === "project_context"
          ) {
            break;
          }

          let targetToolId: string | null = null;

          if (
            nextMsg.tool_call_id &&
            eligibleToolIds.has(nextMsg.tool_call_id)
          ) {
            targetToolId = nextMsg.tool_call_id;
          } else if (!nextMsg.tool_call_id && lastEligibleToolId) {
            targetToolId = lastEligibleToolId;
          }

          if (targetToolId) {
            contextFilesByToolId[targetToolId] = (
              contextFilesByToolId[targetToolId] ?? []
            ).concat(nextMsg.content);
          } else {
            const ctxKey = getMessageKey(nextMsg, j);
            contextFilesAfter.push(
              <ContextFiles
                key={ctxKey}
                files={nextMsg.content}
                toolCallId={nextMsg.tool_call_id}
              />,
            );
          }
          j++;
          continue;
        }

        if (isDiffMessage(nextMsg)) {
          if (nextMsg.tool_call_id && editToolIds.has(nextMsg.tool_call_id)) {
            diffsByToolId[nextMsg.tool_call_id] = (
              diffsByToolId[nextMsg.tool_call_id] ?? []
            ).concat(nextMsg.content);
          } else {
            diffMessagesAfter.push(nextMsg);
          }
          j++;
          continue;
        }

        break;
      }

      const cleanContextFilesByToolId: Record<string, ChatContextFile[]> = {};
      const cleanDiffsByToolId: Record<string, DiffChunk[]> = {};
      for (const [toolId, files] of Object.entries(contextFilesByToolId)) {
        if (files !== undefined) {
          cleanContextFilesByToolId[toolId] = files;
        }
      }
      for (const [toolId, diffs] of Object.entries(diffsByToolId)) {
        if (diffs !== undefined) {
          cleanDiffsByToolId[toolId] = diffs;
        }
      }

      // Only the last assistant message can be actively streaming
      const isLastAssistantMessage =
        i === messages.length - 1 ||
        !messages.slice(i + 1).some((m) => m.role === "assistant");
      const messageIsStreaming = isStreaming && isLastAssistantMessage;

      nodes.push(
        <AssistantInput
          key={key}
          message={head.content}
          reasoningContent={head.reasoning_content}
          thinkingBlocks={head.thinking_blocks}
          toolCalls={head.tool_calls}
          serverExecutedTools={head.server_executed_tools}
          citations={head.citations}
          messageId={head.message_id}
          onBranch={onBranch}
          onDelete={onDelete}
          contextFilesByToolId={cleanContextFilesByToolId}
          diffsByToolId={cleanDiffsByToolId}
          usage={head.usage}
          metering_coins_prompt={head.metering_coins_prompt}
          metering_coins_generated={head.metering_coins_generated}
          metering_coins_cache_creation={head.metering_coins_cache_creation}
          metering_coins_cache_read={head.metering_coins_cache_read}
          isStreaming={messageIsStreaming}
        />,
      );
      for (const ctxNode of contextFilesAfter) {
        nodes.push(ctxNode);
      }

      if (diffMessagesAfter.length > 0) {
        nodes.push(
          <GroupedDiffs key={`diffs-${key}`} diffs={diffMessagesAfter} />,
        );
      }

      i = j - 1;
      continue;
    }

    if (head.role === "user") {
      if (hiddenQaIndices.has(i)) {
        continue;
      }

      const key = getMessageKey(head, i);

      if (i === lastUserIdx) {
        nodes.push(
          <ScrollAreaWithAnchor.ScrollAnchor
            key={`${key}-anchor`}
            behavior="smooth"
            block="start"
          />,
        );
      }

      nodes.push(
        <UserInput
          onRetry={onRetry}
          key={key}
          messageIndex={i}
          messageId={head.message_id}
          onBranch={onBranch}
          onDelete={onDelete}
        >
          {head.content}
        </UserInput>,
      );
      continue;
    }

    if (isChatContextFileMessage(head)) {
      const key = getMessageKey(head, i);
      nodes.push(
        <ContextFiles
          key={key}
          files={head.content}
          toolCallId={head.tool_call_id}
        />,
      );
      continue;
    }

    if (isSystemMessage(head)) {
      const key = getMessageKey(head, i);
      nodes.push(<SystemPrompt key={key} content={head.content} />);
      continue;
    }

    if (isDiffMessage(head)) {
      const key = getMessageKey(head, i);
      const diffs: DiffMessage[] = [head];
      let j = i + 1;
      while (j < messages.length) {
        const m = messages[j];
        if (isToolMessage(m)) {
          j++;
          continue;
        }
        if (isDiffMessage(m)) {
          diffs.push(m);
          j++;
          continue;
        }
        break;
      }

      nodes.push(<GroupedDiffs key={`diffs-${key}`} diffs={diffs} />);
      i = j - 1;
      continue;
    }
  }

  return nodes;
}
