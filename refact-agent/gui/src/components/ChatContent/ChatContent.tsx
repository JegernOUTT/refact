import React, {
  useCallback,
  useMemo,
  useEffect,
  useState,
  useRef,
} from "react";
import { v4 as uuidv4 } from "uuid";
import { ChatMessages, UserMessage } from "../../services/refact";
import { UserInput } from "./UserInput";
import { ScrollArea } from "../ScrollArea";
import { Flex, Container, Button, Box } from "@radix-ui/themes";
import styles from "./ChatContent.module.css";
import { BuddyPulseContent } from "./BuddyPulseContent";
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
  selectIsCompressingById,
} from "../../features/Chat/Thread/selectors";
import {
  createChatWithId,
  switchToThread,
} from "../../features/Chat/Thread/actions";
import { GroupedDiffs } from "./DiffContent";
import { popBackTo } from "../../features/Pages/pagesSlice";
import { ChatLinks, UncommittedChangesWarning } from "../ChatLinks";
import { PlaceHolderText } from "./PlaceHolderText";
import { SkillActivatedCard } from "./SkillActivatedCard";
import { SkillReportCard } from "./SkillReportCard";
import {
  buildDisplayItems,
  DisplayItem,
  tryIncrementalDisplayItemsUpdate,
} from "./ChatContentDisplayItems";
import { QueuedMessage } from "./QueuedMessage";
import { selectSseStatusForChat } from "../../features/Connection";
import { LogoAnimation } from "../LogoAnimation/LogoAnimation.tsx";
import { ChatLoading } from "./ChatLoading";
import {
  removeMessage,
  branchFromChat,
} from "../../services/refact/chatCommands";
import { selectLspPort, selectApiKey } from "../../features/Config/configSlice";
import { VirtualizedChatList } from "./VirtualizedChatList";
import { useCollapsibleState } from "./useCollapsibleState";
import { useCollapsibleStoreProvider } from "./useCollapsibleStoreProvider";
import { CollapsibleStoreProvider } from "./useStoredOpen";
import { SelectionToolbar } from "./SelectionToolbar";
import { ErrorMessageCard } from "./ErrorMessage";
import { SummarizationMessage as SummarizationMessageCard } from "./SummarizationMessage";
import { PlanBanner } from "./PlanBanner";
import { CompressionProgress } from "./CompressionProgress";

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
  const isCompressing = useAppSelector((s) =>
    selectIsCompressingById(s, renderChatId),
  );
  const snapshotReceived = useAppSelector((s) =>
    selectSnapshotReceivedById(s, renderChatId),
  );
  const thread = useAppSelector((s) => selectThreadById(s, renderChatId));
  const sseStatus = useAppSelector((s) =>
    selectSseStatusForChat(s, renderChatId),
  );

  const isConfig = thread !== null && thread.mode === "configurator";
  const isWaiting = useAppSelector((s) => selectIsWaitingById(s, renderChatId));
  const integrationMeta = useAppSelector(selectIntegration);
  const isWaitingForConfirmation = useAppSelector((s) =>
    selectThreadPauseById(s, renderChatId),
  );
  const lspPort = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);

  const collapsibleState = useCollapsibleState(false);
  const collapsibleStore = useCollapsibleStoreProvider(renderChatId);
  const prevChatIdRef = useRef(renderChatId);
  const prevDisplayMessagesRef = useRef<ChatMessages | null>(null);
  const prevDisplayItemsRef = useRef<DisplayItem[] | null>(null);

  useEffect(() => {
    if (prevChatIdRef.current !== renderChatId) {
      collapsibleState.reset();
      prevDisplayMessagesRef.current = null;
      prevDisplayItemsRef.current = null;
      prevChatIdRef.current = renderChatId;
    }
  }, [renderChatId, collapsibleState]);

  const handleBranch = useCallback(
    (messageId: string) => {
      const newChatId = uuidv4();
      const title = `[branched] ${thread?.title ?? "Chat"}`;

      dispatch(
        createChatWithId({
          id: newChatId,
          title,
          parentId: renderChatId,
          linkType: "branch",
          worktree: thread?.worktree,
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
    [dispatch, thread?.title, thread?.worktree, renderChatId, lspPort, apiKey],
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
    return (
      isConfig &&
      !integrationMeta?.path?.includes("project_summary") &&
      !integrationMeta?.path?.includes("setup")
    );
  }, [isConfig, integrationMeta?.path]);

  useDiffFileReload();

  const showLoading =
    !isCompressing &&
    (switching ||
      (!snapshotReceived && messages.length === 0) ||
      (sseStatus === "connecting" && messages.length === 0));

  const displayItems = useMemo(() => {
    const prevMessages = prevDisplayMessagesRef.current;
    const prevItems = prevDisplayItemsRef.current;

    const incremental = tryIncrementalDisplayItemsUpdate(
      prevMessages,
      messages,
      prevItems,
      isStreaming,
    );

    const nextItems = incremental ?? buildDisplayItems(messages, isStreaming);

    prevDisplayMessagesRef.current = messages;
    prevDisplayItemsRef.current = nextItems;

    return nextItems;
  }, [messages, isStreaming]);

  const augmentedDisplayItems = useMemo(() => {
    if (isCompressing) {
      return [
        ...displayItems,
        {
          type: "compression_progress" as const,
          key: "compression-progress-indicator",
          messageIndex: -1,
        },
      ];
    }
    return displayItems;
  }, [displayItems, isCompressing]);

  const initialScrollIndex = useMemo(() => {
    return augmentedDisplayItems.length > 0
      ? augmentedDisplayItems.length - 1
      : undefined;
  }, [augmentedDisplayItems]);

  const virtuosoFooter = useMemo(
    () => (
      <>
        <Container>
          <UncommittedChangesWarning />
        </Container>
        <Flex justify="center" pt="4" pb="8">
          {!isWaitingForConfirmation && (
            <LogoAnimation
              size="8"
              isStreaming={isStreaming}
              isWaiting={isWaiting}
            />
          )}
        </Flex>
      </>
    ),
    [isStreaming, isWaiting, isWaitingForConfirmation],
  );

  const renderDisplayItem = useCallback(
    (item: DisplayItem): React.ReactNode => {
      switch (item.type) {
        case "plain_text":
          return <PlainText>{item.content}</PlainText>;

        case "error":
          return <ErrorMessageCard errors={item.errors} />;

        case "assistant":
          return (
            <AssistantInput
              message={item.message.content}
              reasoningContent={item.message.reasoning_content}
              thinkingBlocks={item.message.thinking_blocks}
              toolCalls={item.message.tool_calls}
              serverExecutedTools={item.message.server_executed_tools}
              serverContentBlocks={item.message.server_content_blocks}
              citations={item.message.citations}
              messageId={item.message.message_id}
              onBranch={handleBranch}
              onDelete={handleDelete}
              contextFilesByToolId={item.contextFilesByToolId}
              diffsByToolId={item.diffsByToolId}
              usage={item.message.usage}
              isStreaming={item.isStreaming}
            />
          );

        case "user":
          return (
            <UserInput
              onRetry={onRetryWrapper}
              messageIndex={item.index}
              messageId={item.message.message_id}
              checkpoints={item.message.checkpoints}
              onBranch={handleBranch}
              onDelete={handleDelete}
            >
              {item.message.content}
            </UserInput>
          );

        case "context_files": {
          if (item.toolCallId === "buddy_project_memory_pulse") {
            return (
              <BuddyPulseContent key={item.key} rawExtra={item.rawExtra} />
            );
          }

          const stateKey = `context_files:${item.toolCallId ?? item.key}`;
          return (
            <ContextFiles
              files={item.files}
              toolCallId={item.toolCallId}
              open={collapsibleState.isOpen(stateKey)}
              onOpenChange={(open) => collapsibleState.setOpen(stateKey, open)}
            />
          );
        }

        case "diff_group": {
          const stateKey = `diff_group:${item.key}`;
          return (
            <GroupedDiffs
              diffs={item.diffs}
              open={collapsibleState.isOpen(stateKey)}
              onOpenChange={(open) => collapsibleState.setOpen(stateKey, open)}
            />
          );
        }

        case "system":
          return <SystemPrompt content={item.content} />;

        case "skill_activated":
          return (
            <SkillActivatedCard
              key={item.key}
              name={item.name}
              body={item.body}
              allowedTools={item.allowedTools}
              modelOverride={item.modelOverride}
            />
          );

        case "skill_report":
          return (
            <SkillReportCard
              key={item.key}
              skillName={item.skillName}
              report={item.report}
              storeKey={`sr:${item.key}`}
            />
          );

        case "summarization":
          return (
            <SummarizationMessageCard key={item.key} message={item.message} />
          );

        case "compression_progress":
          return <CompressionProgress />;

        default:
          return null;
      }
    },
    [handleBranch, handleDelete, onRetryWrapper, collapsibleState],
  );

  if (showLoading) {
    return (
      <Flex
        direction="column"
        className={styles.content}
        data-element="ChatContent"
        p="2"
        gap="1"
        style={{ flexGrow: 1, height: "100%" }}
      >
        <ChatLoading />
      </Flex>
    );
  }

  if (messages.length === 0 && !isCompressing) {
    return (
      <Flex
        direction="column"
        className={styles.content}
        data-element="ChatContent"
        p="2"
        gap="1"
        style={{ flexGrow: 1, height: "100%" }}
      >
        <Container style={{ height: "100%" }}>
          <PlaceHolderText />
        </Container>
      </Flex>
    );
  }

  return (
    <CollapsibleStoreProvider value={collapsibleStore}>
      <SelectionToolbar />
      <Box
        style={{ flexGrow: 1, height: "100%", position: "relative" }}
        data-element="ChatContent"
      >
        <VirtualizedChatList
          key={renderChatId}
          items={augmentedDisplayItems}
          renderItem={renderDisplayItem}
          initialScrollIndex={initialScrollIndex}
          footer={virtuosoFooter}
          header={<PlanBanner threadId={renderChatId} />}
          isStreaming={isStreaming}
        />

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
      </Box>
    </CollapsibleStoreProvider>
  );
};

ChatContent.displayName = "ChatContent";
