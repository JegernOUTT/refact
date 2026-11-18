import { ContextRebuildBanner } from "./ContextRebuildBanner";
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
import { Flex, Container, Box, Text } from "@radix-ui/themes";
import { Button, ErrorState } from "../ui";
import styles from "./ChatContent.module.css";
import { BuddyPulseContent } from "./BuddyPulseContent";
import { ContextFiles } from "./ContextFiles";
import { SystemPrompt } from "./SystemPrompt";
import { AssistantInput } from "./AssistantInput";
import { Markdown } from "../Markdown";
import {
  isRichReviewContribution,
  ReviewChatContribution,
} from "../../features/Review/ReviewChatContribution";
import { PlainText } from "./PlainText";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useDiffFileReload } from "../../hooks/useDiffFileReload";
import {
  selectIntegrationById,
  selectIsStreamingById,
  selectIsWaitingById,
  selectMessagesById,
  selectSnapshotReceivedById,
  selectSubscriptionFailureById,
  selectThreadModeById,
  selectThreadPauseById,
  selectThreadTitleById,
  selectThreadWorktreeById,
  selectIsCompressingById,
  selectCompressionPhaseById,
  selectCompressionReasonById,
  selectCompressionPulseSeqById,
  selectIsCompanionChat,
} from "../../features/Chat/Thread/selectors";
import {
  createChatWithId,
  requestSseRefresh,
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
import { EventRow } from "./EventRow";
import {
  groupToolRuns,
  isInterruptedAssistant,
  type TranscriptDisplayItem,
} from "./toolRunGroups";
import { ToolRunGroup } from "./ToolRunGroup";
import { revealProcessOutput } from "./revealProcessOutput";
import { selectSseStatusForChat } from "../../features/Connection";
import { LogoAnimation } from "../LogoAnimation/LogoAnimation.tsx";
import { ChatLoading } from "./ChatLoading";
import { ChatShield } from "../../features/Privacy/ChatShield";
import {
  removeMessage,
  branchFromChat,
} from "../../services/refact/chatCommands";
import { ChatThreadProvider, useThreadId } from "../../features/Chat/Thread";
import { selectConfig, selectApiKey } from "../../features/Config/configSlice";
import { VirtualizedChatList } from "./VirtualizedChatList";
import { RecoveryCard } from "./RecoveryCard";
import {
  collectErrorTexts,
  partitionTranscriptErrors,
  recoveryRecordFromError,
  recoveryStatusHosts,
  retryableErrorIndex,
} from "../../services/refact/recoveryPresentation";
import type { ErrorMessage } from "../../services/refact/types";
import {
  selectRecoveryById,
  selectQueuedItemsById,
} from "../../features/Chat/Thread/selectors";
import { useCollapsibleState } from "./useCollapsibleState";
import { useCollapsibleStoreProvider } from "./useCollapsibleStoreProvider";
import { CollapsibleStoreProvider } from "./useStoredOpen";
import { SelectionToolbar } from "./SelectionToolbar";
import { ErrorMessageCard } from "./ErrorMessage";
import { SummarizationMessage as SummarizationMessageCard } from "./SummarizationMessage";
import { PlanBanner } from "./PlanBanner";
import { ToolJobCompletionCard } from "./ToolCard/ToolJobCard";
import { ToolJobResult } from "./ToolCard/ToolJobResult";
import {
  isTerminalToolJob,
  parseToolJobReceipt,
} from "../../services/refact/toolJobs";
import type {
  CompressionPhase,
  CompressionReason,
} from "../../services/refact/chatSubscription";

const COMPRESSION_MIN_VISIBLE_MS = 500;
const ESTIMATED_LINE_HEIGHT_PX = 22;
const ESTIMATED_CHARS_PER_LINE = 90;
const ESTIMATED_TEXT_CHROME_PX = 40;
const ESTIMATED_TOOL_ROW_PX = 70;
const ESTIMATED_USER_MESSAGE_PX = 100;
const ESTIMATED_COMPACT_ROW_PX = 48;
const ESTIMATED_TOOL_RUN_PX = 40;
const MAX_ESTIMATED_ITEM_HEIGHT_PX = 1200;

function estimateTextHeight(text: string): number {
  const lines = text
    .split("\n")
    .reduce(
      (total, line) =>
        total + Math.max(1, Math.ceil(line.length / ESTIMATED_CHARS_PER_LINE)),
      0,
    );
  return Math.min(
    MAX_ESTIMATED_ITEM_HEIGHT_PX,
    ESTIMATED_TEXT_CHROME_PX + lines * ESTIMATED_LINE_HEIGHT_PX,
  );
}

function estimateDisplayItemHeight(item: TranscriptDisplayItem): number {
  switch (item.type) {
    case "tool_run":
      return ESTIMATED_TOOL_RUN_PX;
    case "assistant": {
      const text =
        item.view && !item.view.prose
          ? ""
          : typeof item.message.content === "string"
            ? item.message.content
            : "";
      const toolRows =
        (item.view?.toolCalls.length ?? item.message.tool_calls?.length ?? 0) *
        ESTIMATED_TOOL_ROW_PX;
      if (!text.trim()) return Math.max(toolRows, ESTIMATED_TOOL_ROW_PX);
      return Math.min(
        MAX_ESTIMATED_ITEM_HEIGHT_PX,
        estimateTextHeight(text) + toolRows,
      );
    }
    case "user":
      return typeof item.message.content === "string"
        ? Math.max(
            ESTIMATED_USER_MESSAGE_PX,
            estimateTextHeight(item.message.content),
          )
        : ESTIMATED_USER_MESSAGE_PX;
    case "plain_text":
      return estimateTextHeight(item.content);
    default:
      return ESTIMATED_COMPACT_ROW_PX;
  }
}

function compressionProgressText(
  phase: CompressionPhase | undefined,
  reason: CompressionReason | undefined,
): string {
  if (reason === "provider_length_stop" || reason === "context_length_stop") {
    return "Compacting context after provider limit…";
  }

  if (phase === "checking") return "Checking context size…";
  if (phase === "running") return "Compacting older context…";
  if (phase === "applied") return "Context compacted";
  if (phase === "failed") return "Context compaction failed";
  if (phase === "skipped") {
    if (reason === "pre_compact_denied") {
      return "Compaction blocked by a PreCompact hook";
    }
    if (reason === "no_eligible_segment") return "Nothing safe to compact";
    return "Context already compact enough";
  }

  return "Compacting older context…";
}

export type ChatContentProps = {
  onRetry: (index: number, question: UserMessage["content"]) => void;
  onStopStreaming: () => void;
  onRetryGeneration?: () => void;
};

export const ChatContent: React.FC<ChatContentProps> = ({
  onStopStreaming,
  onRetry,
  onRetryGeneration,
}) => {
  const dispatch = useAppDispatch();
  const chatId = useThreadId();
  const recovery = useAppSelector((state) => selectRecoveryById(state, chatId));
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
  const userSubmissions = useMemo(
    () => [
      ...messages.flatMap((message, index) =>
        message.role === "user"
          ? [`message:${message.message_id ?? index}`]
          : [],
      ),
      ...queuedItems.flatMap((item) =>
        item.command_type === "user_message"
          ? [`queue:${item.client_request_id}`]
          : [],
      ),
    ],
    [messages, queuedItems],
  );
  const seenSubmissions = useRef(new Set(userSubmissions));
  const [submissionVersion, setSubmissionVersion] = useState(0);
  useEffect(() => {
    if (!userSubmissions.some((id) => !seenSubmissions.current.has(id))) return;
    userSubmissions.forEach((id) => seenSubmissions.current.add(id));
    setSubmissionVersion((version) => version + 1);
  }, [userSubmissions]);
  const submissionKey = `${renderChatId}:${submissionVersion}`;
  const { episode: episodeErrors, boundary: turnBoundary } = useMemo(
    () => partitionTranscriptErrors(messages, recovery),
    [messages, recovery],
  );
  const isStreaming = useAppSelector((s) =>
    selectIsStreamingById(s, renderChatId),
  );
  const isCompressing = useAppSelector((s) =>
    selectIsCompressingById(s, renderChatId),
  );
  const compressionPhase = useAppSelector((s) =>
    selectCompressionPhaseById(s, renderChatId),
  );
  const compressionReason = useAppSelector((s) =>
    selectCompressionReasonById(s, renderChatId),
  );
  const compressionPulseSeq = useAppSelector((s) =>
    selectCompressionPulseSeqById(s, renderChatId),
  );
  const compressionActive = isCompressing;
  const snapshotReceived = useAppSelector((s) =>
    selectSnapshotReceivedById(s, renderChatId),
  );
  const subscriptionFailure = useAppSelector((s) =>
    selectSubscriptionFailureById(s, renderChatId),
  );
  const threadMode = useAppSelector((s) =>
    selectThreadModeById(s, renderChatId),
  );
  const isCompanionChat = useAppSelector((s) =>
    selectIsCompanionChat(s, renderChatId),
  );
  const threadTitle = useAppSelector((s) =>
    selectThreadTitleById(s, renderChatId),
  );
  const threadWorktree = useAppSelector((s) =>
    selectThreadWorktreeById(s, renderChatId),
  );
  const sseStatus = useAppSelector((s) =>
    selectSseStatusForChat(s, renderChatId),
  );

  const isConfig = threadMode === "configurator";
  const isWaiting = useAppSelector((s) => selectIsWaitingById(s, renderChatId));
  const integrationMeta = useAppSelector((s) =>
    selectIntegrationById(s, renderChatId),
  );
  const isWaitingForConfirmation = useAppSelector((s) =>
    selectThreadPauseById(s, renderChatId),
  );
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);

  const collapsibleState = useCollapsibleState(false);
  const collapsibleStore = useCollapsibleStoreProvider(renderChatId);
  const prevChatIdRef = useRef(renderChatId);
  const prevDisplayMessagesRef = useRef<ChatMessages | null>(null);
  const prevDisplayItemsRef = useRef<DisplayItem[] | null>(null);
  const prevTranscriptRef = useRef<{
    chatId: string;
    items: TranscriptDisplayItem[];
  } | null>(null);
  const [visibleCompression, setVisibleCompression] =
    useState(compressionActive);
  const compressionVisibleSinceRef = useRef<number | null>(
    compressionActive ? Date.now() : null,
  );
  const handledCompressionPulseSeqRef = useRef<string | undefined>(undefined);
  const compressionPulseActive =
    compressionPulseSeq !== undefined &&
    compressionPulseSeq !== handledCompressionPulseSeqRef.current;
  const compressionStatusText = compressionProgressText(
    compressionPhase,
    compressionReason,
  );

  useEffect(() => {
    if (compressionActive) {
      compressionVisibleSinceRef.current = Date.now();
      setVisibleCompression(true);
      return;
    }

    if (compressionPulseActive) {
      handledCompressionPulseSeqRef.current = compressionPulseSeq;
      if (compressionVisibleSinceRef.current === null) {
        compressionVisibleSinceRef.current = Date.now();
        setVisibleCompression(true);
      }
    }

    const visibleSince = compressionVisibleSinceRef.current;
    if (visibleSince === null) {
      setVisibleCompression(false);
      return;
    }

    const elapsed = Date.now() - visibleSince;
    const remaining = COMPRESSION_MIN_VISIBLE_MS - elapsed;
    if (remaining <= 0) {
      compressionVisibleSinceRef.current = null;
      setVisibleCompression(false);
      return;
    }

    const timeoutId = window.setTimeout(() => {
      compressionVisibleSinceRef.current = null;
      setVisibleCompression(false);
    }, remaining);

    return () => window.clearTimeout(timeoutId);
  }, [compressionActive, compressionPulseActive, compressionPulseSeq]);

  useEffect(() => {
    if (prevChatIdRef.current !== renderChatId) {
      collapsibleState.reset();
      prevDisplayMessagesRef.current = null;
      prevDisplayItemsRef.current = null;
      compressionVisibleSinceRef.current = compressionActive
        ? Date.now()
        : null;
      setVisibleCompression(compressionActive);
      handledCompressionPulseSeqRef.current = compressionPulseSeq;
      prevChatIdRef.current = renderChatId;
    }
  }, [renderChatId, collapsibleState, compressionActive, compressionPulseSeq]);

  const handleBranch = useCallback(
    (messageId: string) => {
      const newChatId = uuidv4();
      const title = `[branched] ${threadTitle ?? "Chat"}`;

      dispatch(
        createChatWithId({
          id: newChatId,
          title,
          parentId: renderChatId,
          linkType: "branch",
          worktree: threadWorktree ?? undefined,
        }),
      );

      dispatch(switchToThread({ id: newChatId }));

      void branchFromChat(
        newChatId,
        renderChatId,
        messageId,
        config,
        apiKey ?? undefined,
      ).catch((err) => {
        // eslint-disable-next-line no-console
        console.error("Failed to branch chat:", err);
      });
    },
    [dispatch, threadTitle, threadWorktree, renderChatId, config, apiKey],
  );

  const handleDelete = useCallback(
    (messageId: string) => {
      void removeMessage(
        renderChatId,
        messageId,
        config,
        apiKey ?? undefined,
      ).catch((err) => {
        // eslint-disable-next-line no-console
        console.error("Failed to delete message:", err);
      });
    },
    [renderChatId, config, apiKey],
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
        projectPath: integrationMeta?.project,
        integrationName: integrationMeta?.name,
        integrationPath: integrationMeta?.path,
        wasOpenedThroughChat: true,
      }),
    );
  }, [
    onStopStreaming,
    dispatch,
    integrationMeta?.project,
    integrationMeta?.name,
    integrationMeta?.path,
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
    (!visibleCompression &&
      !compressionActive &&
      ((!snapshotReceived && messages.length === 0) ||
        (sseStatus === "connecting" && messages.length === 0)));

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

  const transcriptItems = useMemo(() => {
    const previous = prevTranscriptRef.current;
    const items = groupToolRuns(
      displayItems,
      previous?.chatId === renderChatId ? previous.items : [],
    );
    prevTranscriptRef.current = { chatId: renderChatId, items };
    return items;
  }, [displayItems, renderChatId]);

  const initialScrollIndex = useMemo(() => {
    return transcriptItems.length > 0 ? transcriptItems.length - 1 : undefined;
  }, [transcriptItems]);

  const liveRecovery =
    recovery && recovery.state !== "succeeded" ? recovery : null;
  const statusHosts = useMemo(() => recoveryStatusHosts(messages), [messages]);
  const archivedEpisodeRecords = useMemo(
    () =>
      episodeErrors.flatMap((error) => {
        const record = recoveryRecordFromError(error);
        return record && record.state !== "succeeded" ? [record] : [];
      }),
    [episodeErrors],
  );
  const liveHost = useMemo(() => {
    const latest = liveRecovery ? episodeErrors.at(-1) : undefined;
    const item = latest
      ? displayItems.find(
          (candidate) =>
            candidate.type === "error" && candidate.errors.includes(latest),
        )
      : undefined;
    return item?.type === "error" ? item : null;
  }, [liveRecovery, episodeErrors, displayItems]);
  const liveHostIndex = liveHost?.messageIndex ?? -1;
  const transcriptErrors = useMemo(
    () =>
      displayItems.flatMap((item) =>
        item.type === "error" ? item.errors : [],
      ),
    [displayItems],
  );
  const liveCardErrors = useMemo(
    () =>
      episodeErrors.filter((error) =>
        liveHost
          ? liveHost.errors.includes(error)
          : !transcriptErrors.includes(error),
      ),
    [episodeErrors, liveHost, transcriptErrors],
  );
  const earlierEpisodeErrors = useMemo(
    () => episodeErrors.filter((error) => !liveCardErrors.includes(error)),
    [episodeErrors, liveCardErrors],
  );
  const representedErrorTexts = useMemo(
    () =>
      new Set(
        collectErrorTexts(
          transcriptErrors.filter((error) => !liveCardErrors.includes(error)),
        ),
      ),
    [transcriptErrors, liveCardErrors],
  );

  const retryErrorIndex = useMemo(
    () =>
      liveRecovery
        ? -1
        : retryableErrorIndex(
            displayItems.flatMap((item) =>
              item.type === "error" ? [item] : [],
            ),
            turnBoundary,
            episodeErrors,
          ),
    [liveRecovery, displayItems, turnBoundary, episodeErrors],
  );

  const handleProcessCompletedClick = useCallback((processId: string) => {
    revealProcessOutput(processId);
  }, []);

  const virtuosoFooter = useMemo(
    () => (
      <>
        <Container className={styles.transcriptFooter}>
          <UncommittedChangesWarning />
          {liveRecovery && liveHostIndex === -1 && (
            <RecoveryCard
              chatId={chatId}
              recovery={liveRecovery}
              errors={liveCardErrors}
              earlier={earlierEpisodeErrors}
              represented={representedErrorTexts}
              archived={archivedEpisodeRecords}
              submissionKey={submissionKey}
            />
          )}
        </Container>
        <Flex
          justify="center"
          pt="4"
          pb="8"
          direction="column"
          align="center"
          gap="2"
        >
          {!isWaitingForConfirmation && (
            <>
              <LogoAnimation
                size="8"
                isStreaming={isStreaming || visibleCompression}
                isWaiting={isWaiting || visibleCompression}
              />
              {visibleCompression && (
                <Text
                  size="2"
                  role="status"
                  aria-live="polite"
                  data-testid="compression-progress"
                >
                  {compressionStatusText}
                </Text>
              )}
            </>
          )}
        </Flex>
        <div aria-hidden="true" className={styles.composerClearance} />
      </>
    ),
    [
      liveRecovery,
      liveHostIndex,
      liveCardErrors,
      earlierEpisodeErrors,
      representedErrorTexts,
      archivedEpisodeRecords,
      submissionKey,
      chatId,
      compressionStatusText,
      isStreaming,
      isWaiting,
      isWaitingForConfirmation,
      visibleCompression,
    ],
  );

  const renderDisplayItem = useCallback(
    (item: DisplayItem): React.ReactNode => {
      switch (item.type) {
        case "plain_text":
          return <PlainText>{item.content}</PlainText>;

        case "error": {
          const staticCard = (errors: ErrorMessage[]) =>
            errors.length > 0 ? (
              <ErrorMessageCard
                errors={errors}
                statusHosts={statusHosts}
                liveRunId={liveRecovery?.run_id}
                onRetryGeneration={
                  item.messageIndex === retryErrorIndex
                    ? onRetryGeneration
                    : undefined
                }
              />
            ) : null;
          if (!liveRecovery || item.messageIndex !== liveHostIndex)
            return staticCard(item.errors);
          const position = item.errors.indexOf(
            episodeErrors[episodeErrors.length - 1],
          );
          const others = (errors: ErrorMessage[]) =>
            errors.filter((error) => !episodeErrors.includes(error));
          return (
            <>
              {staticCard(others(item.errors.slice(0, position)))}
              <RecoveryCard
                chatId={chatId}
                recovery={liveRecovery}
                errors={liveCardErrors}
                earlier={earlierEpisodeErrors}
                represented={representedErrorTexts}
                archived={archivedEpisodeRecords}
                submissionKey={submissionKey}
              />
              {staticCard(others(item.errors.slice(position + 1)))}
            </>
          );
        }

        case "assistant":
          return (
            <>
              {isInterruptedAssistant(item.message) && (
                <Text size="1">
                  Interrupted output — not included in the next request
                </Text>
              )}
              <AssistantInput
                message={item.message.content}
                bodyContent={
                  isRichReviewContribution(item.message.extra) ? (
                    <ReviewChatContribution
                      extra={item.message.extra}
                      chatId={renderChatId}
                      fallback={<Markdown>{item.message.content}</Markdown>}
                    />
                  ) : undefined
                }
                reasoningContent={item.message.reasoning_content}
                thinkingBlocks={item.message.thinking_blocks}
                view={item.view}
                toolCalls={item.message.tool_calls}
                serverExecutedTools={item.message.server_executed_tools}
                serverContentBlocks={item.message.server_content_blocks}
                citations={item.message.citations}
                messageId={item.message.message_id}
                onBranch={isCompanionChat ? undefined : handleBranch}
                onDelete={isCompanionChat ? undefined : handleDelete}
                contextFilesByToolId={item.contextFilesByToolId}
                diffsByToolId={item.diffsByToolId}
                jobResultsByToolId={item.jobResultsByToolId}
                usage={item.message.usage}
                isStreaming={item.isStreaming}
                threadId={renderChatId}
              />
            </>
          );

        case "user": {
          const job = parseToolJobReceipt(item.message.extra?.tool_job);
          const completion =
            job?.chat_id === renderChatId && isTerminalToolJob(job.status)
              ? job
              : null;
          return (
            <UserInput
              onRetry={
                isCompanionChat || completion ? undefined : onRetryWrapper
              }
              bodyContent={
                completion && typeof item.message.content === "string" ? (
                  <>
                    {isRichReviewContribution(item.message.extra) ? (
                      <ReviewChatContribution
                        extra={item.message.extra}
                        chatId={renderChatId}
                        fallback={<Markdown>{item.message.content}</Markdown>}
                      />
                    ) : (
                      <Markdown>{item.message.content}</Markdown>
                    )}
                    <ToolJobCompletionCard receipt={completion} />
                  </>
                ) : typeof item.message.content === "string" &&
                  isRichReviewContribution(item.message.extra) ? (
                  <ReviewChatContribution
                    extra={item.message.extra}
                    chatId={renderChatId}
                    fallback={<Markdown>{item.message.content}</Markdown>}
                  />
                ) : undefined
              }
              messageIndex={item.index}
              messageId={item.message.message_id}
              checkpoints={item.message.checkpoints}
              onBranch={isCompanionChat ? undefined : handleBranch}
              onDelete={isCompanionChat ? undefined : handleDelete}
            >
              {item.message.content}
            </UserInput>
          );
        }

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

        case "tool_job_result":
          return <ToolJobResult key={item.key} result={item.result} />;

        case "event":
          return (
            <EventRow
              key={item.key}
              event={item.event}
              run={item.run}
              onOpenProcessOutput={handleProcessCompletedClick}
            />
          );

        default:
          return null;
      }
    },
    [
      handleBranch,
      handleDelete,
      handleProcessCompletedClick,
      onRetryWrapper,
      collapsibleState,
      renderChatId,
      onRetryGeneration,
      episodeErrors,
      retryErrorIndex,
      statusHosts,
      liveRecovery,
      liveHostIndex,
      liveCardErrors,
      earlierEpisodeErrors,
      representedErrorTexts,
      archivedEpisodeRecords,
      submissionKey,
      chatId,
      isCompanionChat,
    ],
  );

  const renderTranscriptItem = useCallback(
    (item: TranscriptDisplayItem): React.ReactNode =>
      item.type === "tool_run" ? (
        <ToolRunGroup
          group={item}
          threadId={renderChatId}
          renderMember={renderDisplayItem}
        />
      ) : (
        renderDisplayItem(item)
      ),
    [renderChatId, renderDisplayItem],
  );

  if (subscriptionFailure && !snapshotReceived && messages.length === 0) {
    return (
      <Flex
        direction="column"
        className={`${styles.content} ${styles.chatColumn}`}
        data-element="ChatContent"
        p="2"
        gap="1"
      >
        <ErrorState
          variant="full"
          title="This chat could not be opened"
          description={subscriptionFailure}
          data-testid="chat-subscription-failure"
          retry={
            <Button
              variant="soft"
              onClick={() =>
                dispatch(requestSseRefresh({ chatId: renderChatId }))
              }
            >
              Retry
            </Button>
          }
        />
      </Flex>
    );
  }

  if (showLoading) {
    return (
      <Flex
        direction="column"
        className={`${styles.content} ${styles.chatColumn}`}
        data-element="ChatContent"
        p="2"
        gap="1"
      >
        <ChatLoading />
      </Flex>
    );
  }

  if (
    messages.length === 0 &&
    !visibleCompression &&
    (!recovery || recovery.state === "succeeded")
  ) {
    return (
      <Flex
        direction="column"
        className={`${styles.content} ${styles.chatColumn}`}
        data-element="ChatContent"
        p="2"
        gap="1"
      >
        <Container height="100%">
          <PlaceHolderText />
        </Container>
      </Flex>
    );
  }

  return (
    <ChatThreadProvider chatId={renderChatId}>
      <CollapsibleStoreProvider value={collapsibleStore}>
        <SelectionToolbar />
        <Flex
          direction="column"
          className={styles.chatRoot}
          data-element="ChatContent"
        >
          <Box className={styles.transcriptScrollRegion}>
            <VirtualizedChatList
              key={renderChatId}
              items={transcriptItems}
              renderItem={renderTranscriptItem}
              estimateItemHeight={estimateDisplayItemHeight}
              initialScrollIndex={initialScrollIndex}
              footer={virtuosoFooter}
              header={
                <Flex direction="column" gap="2">
                  <ContextRebuildBanner chatId={renderChatId} />
                  <ChatShield threadId={renderChatId} />
                  <PlanBanner threadId={renderChatId} />
                </Flex>
              }
              isStreaming={isStreaming}
            />

            <Box className={styles.floatingLinks}>
              <ScrollArea scrollbars="horizontal">
                <Flex align="start" gap="3" pb="2">
                  {shouldConfigButtonBeVisible && (
                    <Button
                      title="Return to configuration page"
                      onClick={handleReturnToConfigurationClick}
                      size="sm"
                      variant="soft"
                    >
                      Return
                    </Button>
                  )}
                  <ChatLinks />
                </Flex>
              </ScrollArea>
            </Box>
          </Box>
        </Flex>
      </CollapsibleStoreProvider>
    </ChatThreadProvider>
  );
};

ChatContent.displayName = "ChatContent";
