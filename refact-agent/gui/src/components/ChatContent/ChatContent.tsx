import React, {
  useCallback,
  useMemo,
  useEffect,
  useState,
  useRef,
} from "react";
import { v4 as uuidv4 } from "uuid";
import {
  AssistantMessage,
  ChatContextFile,
  ChatMessages,
  DiffChunk,
  DiffMessage,
  ErrorMessage,
  isChatContextFileMessage,
  isDiffMessage,
  isAssistantMessage,
  isEventMessage,
  isErrorMessage,
  isToolMessage,
  isSystemMessage,
  isSummarizationMessage,
  SummarizationMessage,
  UserMessage,
} from "../../services/refact";
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
import { normalizeToolName, isToolName } from "../../utils/toolNameAliases";
import { isSkillReportContent, parseSkillReport } from "./skillReportUtils";
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
    switching ||
    (!snapshotReceived && messages.length === 0) ||
    (sseStatus === "connecting" && messages.length === 0);

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

  const initialScrollIndex = useMemo(() => {
    return displayItems.length > 0 ? displayItems.length - 1 : undefined;
  }, [displayItems]);

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

        default:
          return null;
      }
    },
    [
      handleBranch,
      handleDelete,
      onRetryWrapper,
      collapsibleState,
    ],
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

  if (messages.length === 0) {
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
          items={displayItems}
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

function getMessageKey(message: ChatMessages[number], index: number): string {
  if (message.message_id) return message.message_id;
  if ("tool_call_id" in message && message.tool_call_id) {
    return `${message.role}-${message.tool_call_id}-${index}`;
  }
  return `${message.role}-${index}`;
}

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
  // CC OAuth mode aliases
  "t_cat",
  "t_tree",
  "t_search_pattern",
  "t_search_semantic",
  "t_search_symbol_definition",
  "t_web",
  "t_web_search",
  "t_knowledge",
  "t_hist_search",
  "t_hist_get",
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
  // CC OAuth mode aliases
  "t_write",
  "t_patch",
  "t_patch_re",
  "t_patch_ln",
  "t_patch_at",
  "t_undo",
  "t_rm",
]);

type DisplayItemAssistant = {
  type: "assistant";
  key: string;
  index: number;
  messageIndex: number;
  message: AssistantMessage;
  contextFilesByToolId: Record<string, ChatContextFile[]>;
  diffsByToolId: Record<string, DiffChunk[]>;
  isStreaming: boolean;
};

type DisplayItemUser = {
  type: "user";
  key: string;
  index: number;
  messageIndex: number;
  message: UserMessage;
  isLastUser: boolean;
};

type DisplayItemContextFiles = {
  type: "context_files";
  key: string;
  messageIndex: number;
  files: ChatContextFile[];
  toolCallId?: string;
  rawExtra?: unknown;
};

type DisplayItemDiffGroup = {
  type: "diff_group";
  key: string;
  messageIndex: number;
  diffs: DiffMessage[];
};

type DisplayItemSystem = {
  type: "system";
  key: string;
  messageIndex: number;
  content: string;
};

type DisplayItemPlainText = {
  type: "plain_text";
  key: string;
  messageIndex: number;
  content: string;
};

type DisplayItemError = {
  type: "error";
  key: string;
  messageIndex: number;
  errors: ErrorMessage[];
};

type DisplayItemSkillActivated = {
  type: "skill_activated";
  key: string;
  messageIndex: number;
  name: string;
  body: string;
  allowedTools: string[];
  modelOverride: string | null;
};

type DisplayItemSkillReport = {
  type: "skill_report";
  key: string;
  messageIndex: number;
  skillName: string;
  report: string;
};

type DisplayItemSummarization = {
  type: "summarization";
  key: string;
  messageIndex: number;
  message: SummarizationMessage;
};

type DisplayItem =
  | DisplayItemAssistant
  | DisplayItemUser
  | DisplayItemContextFiles
  | DisplayItemDiffGroup
  | DisplayItemSystem
  | DisplayItemPlainText
  | DisplayItemError
  | DisplayItemSkillActivated
  | DisplayItemSkillReport
  | DisplayItemSummarization;

function updateAssistantStreamingFlags(
  items: DisplayItem[],
  isStreaming: boolean,
  lastAssistantIdx: number,
): DisplayItem[] {
  let needsPatch = false;
  for (const item of items) {
    if (item.type !== "assistant") continue;
    const shouldStream = isStreaming && item.index === lastAssistantIdx;
    if (shouldStream !== item.isStreaming) {
      needsPatch = true;
      break;
    }
  }

  if (!needsPatch) return items;

  return items.map((item) => {
    if (item.type !== "assistant") return item;
    const shouldStream = isStreaming && item.index === lastAssistantIdx;
    return shouldStream === item.isStreaming
      ? item
      : { ...item, isStreaming: shouldStream };
  });
}

function findLastAssistantMessageIndex(messages: ChatMessages): number {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === "assistant") return i;
  }
  return -1;
}

function findRebuildStartIndex(messages: ChatMessages, index: number): number {
  let rebuildStart = index;
  let sawSupplemental = false;

  for (let i = index - 1; i >= 0; i--) {
    const msg = messages[i];

    if (isEventMessage(msg) || msg.role === "plan") {
      rebuildStart = i;
      continue;
    }

    if (
      isToolMessage(msg) ||
      isChatContextFileMessage(msg) ||
      isDiffMessage(msg)
    ) {
      sawSupplemental = true;
      rebuildStart = i;
      continue;
    }

    if (msg.role === "assistant") {
      return i;
    }

    return sawSupplemental ? rebuildStart : index;
  }

  return sawSupplemental ? rebuildStart : index;
}

function rebuildDisplayItemsFromStart(
  previousItems: DisplayItem[],
  nextMessages: ChatMessages,
  isStreaming: boolean,
  rebuildStart: number,
): DisplayItem[] {
  const hidden = computeHiddenQaMessageIndices(nextMessages);
  const tailItems = buildDisplayItemsFromIndex(
    nextMessages,
    isStreaming,
    hidden,
    rebuildStart,
  );
  const prefixItems = previousItems.filter(
    (item) => item.messageIndex < rebuildStart,
  );

  return updateAssistantStreamingFlags(
    [...prefixItems, ...tailItems],
    isStreaming,
    findLastAssistantMessageIndex(nextMessages),
  );
}

function assistantGroupingSignature(message: AssistantMessage): string {
  return (message.tool_calls ?? [])
    .map((toolCall) => {
      const name = normalizeToolName(toolCall.function.name) ?? "";
      const id = toolCall.id ?? "";
      return `${id}:${name}`;
    })
    .join("|");
}

function buildDisplayItemsFromIndex(
  messages: ChatMessages,
  isStreaming: boolean,
  hiddenQaIndices: Set<number>,
  startIndex: number,
): DisplayItem[] {
  const items: DisplayItem[] = [];
  if (startIndex >= messages.length) return items;

  let lastUserIdx = -1;
  let lastAssistantIdx = -1;
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (msg.role === "user" && !hiddenQaIndices.has(i) && lastUserIdx === -1) {
      lastUserIdx = i;
    }
    if (msg.role === "assistant" && lastAssistantIdx === -1) {
      lastAssistantIdx = i;
    }
    if (lastUserIdx !== -1 && lastAssistantIdx !== -1) break;
  }

  for (let i = startIndex; i < messages.length; i++) {
    const head = messages[i];

    if (isToolMessage(head)) continue;
    if (isEventMessage(head)) continue;
    if (head.role === "plan") continue;

    if (isErrorMessage(head)) {
      const errors = [head];
      let j = i + 1;
      while (j < messages.length) {
        const candidate = messages[j];
        if (!isErrorMessage(candidate)) break;
        errors.push(candidate);
        j++;
      }
      items.push({
        type: "error",
        key: getMessageKey(head, i),
        messageIndex: i,
        errors,
      });
      i = j - 1;
      continue;
    }

    if (head.role === "plain_text") {
      if (
        typeof head.content === "string" &&
        isSkillReportContent(head.content)
      ) {
        const parsed = parseSkillReport(head.content);
        if (parsed) {
          items.push({
            type: "skill_report",
            key: getMessageKey(head, i),
            messageIndex: i,
            skillName: parsed.skillName,
            report: parsed.report,
          });
          continue;
        }
      }
      items.push({
        type: "plain_text",
        key: getMessageKey(head, i),
        messageIndex: i,
        content: head.content,
      });
      continue;
    }

    if (head.role === "assistant") {
      const toolCalls = "tool_calls" in head ? head.tool_calls ?? [] : [];
      const isOnlyActivateSkill =
        toolCalls.length > 0 &&
        toolCalls.every((tc) =>
          isToolName(tc.function.name, "activate_skill"),
        ) &&
        !("content" in head && head.content && String(head.content).trim());
      if (isOnlyActivateSkill) {
        continue;
      }

      const key = getMessageKey(head, i);
      const contextFilesAfter: DisplayItemContextFiles[] = [];
      const diffMessagesAfter: DiffMessage[] = [];
      let diffGroupStartIndex: number | null = null;
      const contextFilesByToolId: Partial<Record<string, ChatContextFile[]>> =
        {};
      const diffsByToolId: Partial<Record<string, DiffChunk[]>> = {};

      const eligibleToolCalls = toolCalls.filter(
        (tc) =>
          tc.id &&
          tc.function.name &&
          READ_TOOLS.has(normalizeToolName(tc.function.name) ?? ""),
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
        (tc) =>
          tc.id &&
          tc.function.name &&
          EDIT_TOOLS.has(normalizeToolName(tc.function.name) ?? ""),
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
            const prev = contextFilesByToolId[targetToolId] ?? [];
            contextFilesByToolId[targetToolId] = [...prev, ...nextMsg.content];
          } else {
            contextFilesAfter.push({
              type: "context_files",
              key: getMessageKey(nextMsg, j),
              messageIndex: j,
              files: nextMsg.content,
              toolCallId: nextMsg.tool_call_id,
              rawExtra: (nextMsg as { extra?: unknown }).extra,
            });
          }
          j++;
          continue;
        }

        if (isDiffMessage(nextMsg)) {
          if (nextMsg.tool_call_id && editToolIds.has(nextMsg.tool_call_id)) {
            const prevDiffs = diffsByToolId[nextMsg.tool_call_id] ?? [];
            diffsByToolId[nextMsg.tool_call_id] = [
              ...prevDiffs,
              ...nextMsg.content,
            ];
          } else {
            if (diffGroupStartIndex === null) diffGroupStartIndex = j;
            diffMessagesAfter.push(nextMsg);
          }
          j++;
          continue;
        }

        break;
      }

      items.push({
        type: "assistant",
        key,
        index: i,
        messageIndex: i,
        message: head,
        contextFilesByToolId: contextFilesByToolId as Record<
          string,
          ChatContextFile[]
        >,
        diffsByToolId: diffsByToolId as Record<string, DiffChunk[]>,
        isStreaming: isStreaming && i === lastAssistantIdx,
      });

      for (const ctxItem of contextFilesAfter) {
        items.push(ctxItem);
      }

      if (diffMessagesAfter.length > 0) {
        items.push({
          type: "diff_group",
          key: `diffs-${key}`,
          messageIndex: diffGroupStartIndex ?? i,
          diffs: diffMessagesAfter,
        });
      }

      i = j - 1;
      continue;
    }

    if (head.role === "user") {
      if (hiddenQaIndices.has(i)) {
        continue;
      }

      items.push({
        type: "user",
        key: getMessageKey(head, i),
        index: i,
        messageIndex: i,
        message: head,
        isLastUser: i === lastUserIdx,
      });
      continue;
    }

    if (head.role === "cd_instruction" && typeof head.content === "string") {
      const parsed = tryParseSkillActivated(head.content);
      if (parsed) {
        items.push({
          type: "skill_activated",
          key: getMessageKey(head, i),
          messageIndex: i,
          ...parsed,
        });
      }
      continue;
    }

    if (isSummarizationMessage(head)) {
      items.push({
        type: "summarization",
        key: getMessageKey(head, i),
        messageIndex: i,
        message: head,
      });
      continue;
    }

    if (isChatContextFileMessage(head)) {
      items.push({
        type: "context_files",
        key: getMessageKey(head, i),
        messageIndex: i,
        files: head.content,
        toolCallId: head.tool_call_id,
        rawExtra: (head as { extra?: unknown }).extra,
      });
      continue;
    }

    if (isSystemMessage(head)) {
      items.push({
        type: "system",
        key: getMessageKey(head, i),
        messageIndex: i,
        content: head.content,
      });
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

      items.push({
        type: "diff_group",
        key: `diffs-${key}`,
        messageIndex: i,
        diffs,
      });
      i = j - 1;
      continue;
    }
  }

  return items;
}

function patchTailDisplayItems(
  previousMessages: ChatMessages,
  nextMessages: ChatMessages,
  previousItems: DisplayItem[],
  isStreaming: boolean,
): DisplayItem[] | null {
  if (previousMessages.length >= nextMessages.length) return null;

  const sharedLen = previousMessages.length;
  for (let i = 0; i < sharedLen; i++) {
    if (previousMessages[i] !== nextMessages[i]) return null;
  }

  const rebuildStart = findRebuildStartIndex(nextMessages, sharedLen);
  const nextItems = rebuildDisplayItemsFromStart(
    previousItems,
    nextMessages,
    isStreaming,
    rebuildStart,
  );

  return nextItems;
}

function tryParseSkillActivated(
  content: string,
): Omit<DisplayItemSkillActivated, "type" | "key" | "messageIndex"> | null {
  const prefix = "💿 SKILL_ACTIVATED ";
  const firstNewline = content.indexOf("\n");
  const headerLine =
    firstNewline === -1 ? content : content.slice(0, firstNewline);
  if (!headerLine.startsWith(prefix)) return null;
  try {
    const meta = JSON.parse(headerLine.slice(prefix.length)) as {
      name?: string;
      allowed_tools?: string[];
      model_override?: string | null;
    };
    const body =
      firstNewline === -1 ? "" : content.slice(firstNewline + 1).trimStart();
    return {
      name: meta.name ?? "",
      body,
      allowedTools: meta.allowed_tools ?? [],
      modelOverride: meta.model_override ?? null,
    };
  } catch {
    return null;
  }
}

function buildDisplayItems(
  messages: ChatMessages,
  isStreaming: boolean,
): DisplayItem[] {
  const items: DisplayItem[] = [];
  if (messages.length === 0) return items;

  const hiddenQaIndices = computeHiddenQaMessageIndices(messages);

  let lastUserIdx = -1;
  let lastAssistantIdx = -1;
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i];
    if (msg.role === "user" && !hiddenQaIndices.has(i) && lastUserIdx === -1) {
      lastUserIdx = i;
    }
    if (msg.role === "assistant" && lastAssistantIdx === -1) {
      lastAssistantIdx = i;
    }
    if (lastUserIdx !== -1 && lastAssistantIdx !== -1) break;
  }

  for (let i = 0; i < messages.length; i++) {
    const head = messages[i];

    if (isToolMessage(head)) continue;
    if (isEventMessage(head)) continue;
    if (head.role === "plan") continue;

    if (isErrorMessage(head)) {
      const errors = [head];
      let j = i + 1;
      while (j < messages.length) {
        const candidate = messages[j];
        if (!isErrorMessage(candidate)) break;
        errors.push(candidate);
        j++;
      }
      items.push({
        type: "error",
        key: getMessageKey(head, i),
        messageIndex: i,
        errors,
      });
      i = j - 1;
      continue;
    }

    if (head.role === "plain_text") {
      if (
        typeof head.content === "string" &&
        isSkillReportContent(head.content)
      ) {
        const parsed = parseSkillReport(head.content);
        if (parsed) {
          items.push({
            type: "skill_report",
            key: getMessageKey(head, i),
            messageIndex: i,
            skillName: parsed.skillName,
            report: parsed.report,
          });
          continue;
        }
      }
      items.push({
        type: "plain_text",
        key: getMessageKey(head, i),
        messageIndex: i,
        content: head.content,
      });
      continue;
    }

    if (head.role === "assistant") {
      const toolCalls = "tool_calls" in head ? head.tool_calls ?? [] : [];
      const isOnlyActivateSkill =
        toolCalls.length > 0 &&
        toolCalls.every((tc) =>
          isToolName(tc.function.name, "activate_skill"),
        ) &&
        !("content" in head && head.content && String(head.content).trim());
      if (isOnlyActivateSkill) {
        continue;
      }

      const key = getMessageKey(head, i);
      const contextFilesAfter: DisplayItemContextFiles[] = [];
      const diffMessagesAfter: DiffMessage[] = [];
      let diffGroupStartIndex: number | null = null;
      const contextFilesByToolId: Record<string, ChatContextFile[]> = {};
      const diffsByToolId: Record<string, DiffChunk[]> = {};

      const eligibleToolCalls = toolCalls.filter(
        (tc) =>
          tc.id &&
          tc.function.name &&
          READ_TOOLS.has(normalizeToolName(tc.function.name) ?? ""),
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
        (tc) =>
          tc.id &&
          tc.function.name &&
          EDIT_TOOLS.has(normalizeToolName(tc.function.name) ?? ""),
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
            // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
            const prev = contextFilesByToolId[targetToolId] || [];
            contextFilesByToolId[targetToolId] = [...prev, ...nextMsg.content];
          } else {
            contextFilesAfter.push({
              type: "context_files",
              key: getMessageKey(nextMsg, j),
              messageIndex: j,
              files: nextMsg.content,
              toolCallId: nextMsg.tool_call_id,
              rawExtra: (nextMsg as { extra?: unknown }).extra,
            });
          }
          j++;
          continue;
        }

        if (isDiffMessage(nextMsg)) {
          if (nextMsg.tool_call_id && editToolIds.has(nextMsg.tool_call_id)) {
            // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
            const prevDiffs = diffsByToolId[nextMsg.tool_call_id] || [];
            diffsByToolId[nextMsg.tool_call_id] = [
              ...prevDiffs,
              ...nextMsg.content,
            ];
          } else {
            if (diffGroupStartIndex === null) diffGroupStartIndex = j;
            diffMessagesAfter.push(nextMsg);
          }
          j++;
          continue;
        }

        break;
      }

      items.push({
        type: "assistant",
        key,
        index: i,
        messageIndex: i,
        message: head,
        contextFilesByToolId,
        diffsByToolId,
        isStreaming: isStreaming && i === lastAssistantIdx,
      });

      for (const ctxItem of contextFilesAfter) {
        items.push(ctxItem);
      }

      if (diffMessagesAfter.length > 0) {
        items.push({
          type: "diff_group",
          key: `diffs-${key}`,
          messageIndex: diffGroupStartIndex ?? i,
          diffs: diffMessagesAfter,
        });
      }

      i = j - 1;
      continue;
    }

    if (head.role === "user") {
      if (hiddenQaIndices.has(i)) {
        continue;
      }

      items.push({
        type: "user",
        key: getMessageKey(head, i),
        index: i,
        messageIndex: i,
        message: head,
        isLastUser: i === lastUserIdx,
      });
      continue;
    }

    if (head.role === "cd_instruction" && typeof head.content === "string") {
      const parsed = tryParseSkillActivated(head.content);
      if (parsed) {
        items.push({
          type: "skill_activated",
          key: getMessageKey(head, i),
          messageIndex: i,
          ...parsed,
        });
      }
      continue;
    }

    if (isSummarizationMessage(head)) {
      items.push({
        type: "summarization",
        key: getMessageKey(head, i),
        messageIndex: i,
        message: head,
      });
      continue;
    }

    if (isChatContextFileMessage(head)) {
      items.push({
        type: "context_files",
        key: getMessageKey(head, i),
        messageIndex: i,
        files: head.content,
        toolCallId: head.tool_call_id,
        rawExtra: (head as { extra?: unknown }).extra,
      });
      continue;
    }

    if (isSystemMessage(head)) {
      items.push({
        type: "system",
        key: getMessageKey(head, i),
        messageIndex: i,
        content: head.content,
      });
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

      items.push({
        type: "diff_group",
        key: `diffs-${key}`,
        messageIndex: i,
        diffs,
      });
      i = j - 1;
      continue;
    }
  }

  return items;
}

function tryIncrementalDisplayItemsUpdate(
  previousMessages: ChatMessages | null,
  nextMessages: ChatMessages,
  previousItems: DisplayItem[] | null,
  isStreaming: boolean,
): DisplayItem[] | null {
  if (!previousMessages || !previousItems) return null;
  if (previousMessages.length !== nextMessages.length) {
    return patchTailDisplayItems(
      previousMessages,
      nextMessages,
      previousItems,
      isStreaming,
    );
  }

  let changedIndex = -1;
  for (let i = 0; i < nextMessages.length; i++) {
    if (previousMessages[i] !== nextMessages[i]) {
      if (changedIndex !== -1) return null;
      changedIndex = i;
    }
  }

  let lastAssistantIdx = -1;
  for (let i = nextMessages.length - 1; i >= 0; i--) {
    if (nextMessages[i].role === "assistant") {
      lastAssistantIdx = i;
      break;
    }
  }

  if (changedIndex === -1) {
    return updateAssistantStreamingFlags(
      previousItems,
      isStreaming,
      lastAssistantIdx,
    );
  }

  const changedMessage = nextMessages[changedIndex];
  if (changedMessage.role !== "assistant") return null;
  if (!isAssistantMessage(changedMessage)) return null;

  const previousMessage = previousMessages[changedIndex];
  if (
    !isAssistantMessage(previousMessage) ||
    assistantGroupingSignature(previousMessage) !==
      assistantGroupingSignature(changedMessage)
  ) {
    return rebuildDisplayItemsFromStart(
      previousItems,
      nextMessages,
      isStreaming,
      changedIndex,
    );
  }

  const nextItems = previousItems.map((item) => {
    if (item.type !== "assistant") return item;
    if (item.index !== changedIndex) {
      const shouldStream = isStreaming && item.index === lastAssistantIdx;
      return shouldStream === item.isStreaming
        ? item
        : {
            ...item,
            isStreaming: shouldStream,
          };
    }

    return {
      ...item,
      message: changedMessage,
      isStreaming: isStreaming && item.index === lastAssistantIdx,
    };
  });

  return nextItems;
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
        if (isToolName(tc.function.name, "ask_questions") && tc.id) {
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
