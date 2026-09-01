import React, { useCallback, useEffect, useRef, useState } from "react";
import { Bot } from "lucide-react";
import { ChatForm, ChatFormProps } from "../ChatForm";
import { ChatContent } from "../ChatContent";
import { Flex, Button, Card, Container } from "@radix-ui/themes";
import styles from "./Chat.module.css";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useChatActions } from "../../hooks/useChatActions";
import { type Config } from "../../features/Config/configSlice";
import {
  enableSend,
  flattenBackgroundAgentTree,
  selectActiveBackgroundAgents,
  selectBackgroundAgentsTree,
  selectIsStreamingById,
  selectPreventSendById,
  selectIsBuddyChat,
  switchToThread,
  useThreadId,
} from "../../features/Chat/Thread";
import { BuddyChatCompanion } from "../../features/Buddy";
import { DropzoneProvider } from "../Dropzone";
import { useCheckpoints } from "../../hooks/useCheckpoints";
import { Checkpoints } from "../../features/Checkpoints";
import { TaskProgressWidget } from "../TaskProgressWidget";
import { BrowserContextGuard } from "../../features/Browser/BrowserContextGuard";
import { selectBrowserContextOversize } from "../../features/Browser/browserSlice";
import { SkillsIndicator } from "../ChatContent/SkillsIndicator";
import {
  registerVisibleChatMount,
  unregisterVisibleChatMount,
} from "../../features/Connection";
import {
  selectCapabilities,
  selectHost,
} from "../../features/Config/configSlice";
import { TerminalPanel } from "../../features/Workspace/TerminalPanel";
import { resolveWorkspaceDockAvailability } from "../../features/Workspace/workspaceAvailability";
import {
  selectPanelsForced,
  selectWorkspaceDock,
} from "../../features/Workspace/workspaceSlice";
import { useBottomDockClearance } from "./useBottomDockClearance";
import { AgentsPanel } from "../../features/AgentsPanel";
import {
  autoOpenRequested,
  panelAutoClosed,
  panelOpened,
  selectAgentsPanelOpen,
  selectAgentsPanelUserOpened,
} from "../../features/AgentsPanel/agentsPanelSlice";
import { useMediaQuery } from "../ui";

export type ChatProps = {
  host: Config["host"];
  tabbed: Config["tabbed"];
  backFromChat: () => void;
  style?: React.CSSProperties;
  unCalledTools: boolean;
  maybeSendToSidebar: ChatFormProps["onClose"];
};

export const Chat: React.FC<ChatProps> = ({
  style,
  unCalledTools,
  maybeSendToSidebar,
}) => {
  const dispatch = useAppDispatch();

  const [isViewingRawJSON, setIsViewingRawJSON] = useState(false);
  const chatId = useThreadId();
  const isNarrow = useMediaQuery("(max-width: 719px)");
  const panelOpen = useAppSelector((state) =>
    selectAgentsPanelOpen(state, chatId),
  );
  const panelUserOpened = useAppSelector((state) =>
    selectAgentsPanelUserOpened(state, chatId),
  );
  const agentTree = useAppSelector((state) =>
    selectBackgroundAgentsTree(state, chatId),
  );
  const agents = flattenBackgroundAgentTree(agentTree);
  const activeAgents = useAppSelector((state) =>
    selectActiveBackgroundAgents(state, chatId),
  );
  const previousRunningCount = useRef<number | null>(null);
  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, chatId),
  );
  const isBuddyChat = useAppSelector((state) =>
    selectIsBuddyChat(state, chatId),
  );
  const browserOversizeInfo = useAppSelector((state) =>
    selectBrowserContextOversize(state, chatId),
  );
  const host = useAppSelector(selectHost);
  const capabilities = useAppSelector(selectCapabilities);
  const panelsForced = useAppSelector(selectPanelsForced);
  const workspaceDock = useAppSelector(selectWorkspaceDock);
  const workspaceAvailability = resolveWorkspaceDockAvailability(
    host,
    capabilities,
    panelsForced,
  );
  const showTerminalWorkbench =
    workspaceAvailability.terminal &&
    (!workspaceAvailability.dock || workspaceDock.open);

  const { submit, abort, retryFromIndex, regenerate } = useChatActions(chatId);

  const { shouldCheckpointsPopupBeShown } = useCheckpoints();

  useEffect(() => {
    dispatch(registerVisibleChatMount({ chatId }));
    return () => {
      dispatch(unregisterVisibleChatMount({ chatId }));
    };
  }, [dispatch, chatId]);

  useEffect(() => {
    const previous = previousRunningCount.current;
    const running = activeAgents.length;
    previousRunningCount.current = running;

    if (previous === 0 && running > 0 && !isNarrow) {
      dispatch(autoOpenRequested(chatId));
    }
    if (
      previous !== null &&
      previous > 0 &&
      running === 0 &&
      !panelUserOpened
    ) {
      dispatch(panelAutoClosed(chatId));
    }
  }, [activeAgents.length, chatId, dispatch, isNarrow, panelUserOpened]);

  const handleToggleAgents = useCallback(() => {
    if (panelOpen) {
      dispatch(panelAutoClosed(chatId));
    } else {
      dispatch(panelOpened(chatId));
    }
  }, [chatId, dispatch, panelOpen]);

  const handleAgentNavigation = useCallback(
    (childChatId: string) => {
      dispatch(switchToThread({ id: childChatId }));
    },
    [dispatch],
  );

  const preventSend = useAppSelector((state) =>
    selectPreventSendById(state, chatId),
  );
  const onEnableSend = () => dispatch(enableSend({ id: chatId }));

  const bottomDockRef = useRef<HTMLDivElement>(null);
  useBottomDockClearance(bottomDockRef);

  const handleSubmit = useCallback(
    (value: string, sendPolicy?: "immediate" | "after_flow") => {
      const priority = sendPolicy === "immediate";
      void submit(value, priority);
      if (isViewingRawJSON) {
        setIsViewingRawJSON(false);
      }
    },
    [submit, isViewingRawJSON],
  );

  const handleAbort = useCallback(() => {
    void abort();
  }, [abort]);

  const handleRetry = useCallback(
    (index: number, content: Parameters<typeof retryFromIndex>[1]) => {
      void retryFromIndex(index, content);
    },
    [retryFromIndex],
  );

  const handleRetryGeneration = useCallback(() => {
    void regenerate();
  }, [regenerate]);

  return (
    <DropzoneProvider asChild>
      <Flex
        className={styles.chatShell}
        style={{
          ...style,
          minHeight: 0,
          minWidth: 0,
          maxWidth: "100%",
          height: "100%",
          overflow: "hidden",
        }}
      >
        {panelOpen && (
          <AgentsPanel
            chatId={chatId}
            narrow={isNarrow}
            onNavigate={handleAgentNavigation}
          />
        )}
        <Flex
          className={styles.chatRoot}
          direction="column"
          flexGrow="1"
          width="100%"
          px="1"
        >
          {agents.length > 0 && (
            <button
              aria-expanded={panelOpen}
              className={styles.agentsToggle}
              type="button"
              onClick={handleToggleAgents}
            >
              <Bot aria-hidden="true" size={16} />
              <span>Agents</span>
              {activeAgents.length > 0 && (
                <span className={styles.agentsBadge}>
                  {activeAgents.length}
                </span>
              )}
            </button>
          )}
          <Flex
            direction="column"
            className={styles.transcriptArea}
            style={{
              flex: "1 1 auto",
              minHeight: 0,
              minWidth: 0,
              maxWidth: "100%",
              overflow: "hidden",
            }}
          >
            <ChatContent
              onRetry={handleRetry}
              onStopStreaming={handleAbort}
              onRetryGeneration={handleRetryGeneration}
            />
          </Flex>

          <Flex
            ref={bottomDockRef}
            direction="column"
            className={styles.bottomDock}
          >
            <Container>
              <SkillsIndicator chatId={chatId} />
            </Container>

            {!isBuddyChat && shouldCheckpointsPopupBeShown && <Checkpoints />}

            {browserOversizeInfo && (
              <Container>
                <BrowserContextGuard chatId={chatId} />
              </Container>
            )}

            {!isStreaming && preventSend && unCalledTools && (
              <Flex py="4">
                <Card className={styles.dockPanel} style={{ width: "100%" }}>
                  <Flex direction="column" align="center" gap="2" width="100%">
                    Chat was interrupted with uncalled tools calls.
                    <Button onClick={onEnableSend}>Resume</Button>
                  </Flex>
                </Card>
              </Flex>
            )}

            <Container>
              <div className={styles.dockColumn}>
                {!isBuddyChat && <BuddyChatCompanion chatId={chatId} />}
                {showTerminalWorkbench ? (
                  <div className={styles.terminalWorkbench}>
                    <TerminalPanel chatId={chatId} />
                  </div>
                ) : null}
                <div className={styles.dockGroup}>
                  <TaskProgressWidget />
                  <ChatForm
                    key={chatId}
                    embedded
                    onSubmit={handleSubmit}
                    onClose={maybeSendToSidebar}
                  />
                </div>
              </div>
            </Container>
          </Flex>
        </Flex>
      </Flex>
    </DropzoneProvider>
  );
};
