import React, { useCallback, useEffect, useRef, useState } from "react";
import { ChatForm, ChatFormProps } from "../ChatForm";
import { ChatContent } from "../ChatContent";
import { Flex, Button, Card, Container } from "@radix-ui/themes";
import styles from "./Chat.module.css";
import { useAppSelector, useAppDispatch, useChatActions } from "../../hooks";
import { type Config } from "../../features/Config/configSlice";
import {
  enableSend,
  selectIsStreamingById,
  selectPreventSendById,
  selectIsBuddyChat,
  useThreadId,
} from "../../features/Chat/Thread";
import { BuddyChatCompanion } from "../../features/Buddy";
import { DropzoneProvider } from "../Dropzone";
import { useCheckpoints } from "../../hooks/useCheckpoints";
import { Checkpoints } from "../../features/Checkpoints";
import { TaskProgressWidget } from "../TaskProgressWidget";
import { BrowserPanel } from "../../features/Browser/BrowserPanel";
import { BrowserContextGuard } from "../../features/Browser/BrowserContextGuard";
import {
  selectBrowserContextOversize,
  selectBrowserUiOpen,
} from "../../features/Browser/browserSlice";
import { SkillsIndicator } from "../ChatContent/SkillsIndicator";
import {
  registerVisibleChatMount,
  unregisterVisibleChatMount,
} from "../../features/Connection";

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
  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, chatId),
  );
  const isBuddyChat = useAppSelector((state) =>
    selectIsBuddyChat(state, chatId),
  );
  const isBrowserOpen = useAppSelector((state) =>
    selectBrowserUiOpen(state, chatId),
  );
  const browserOversizeInfo = useAppSelector((state) =>
    selectBrowserContextOversize(state, chatId),
  );

  const { submit, abort, retryFromIndex } = useChatActions(chatId);

  const { shouldCheckpointsPopupBeShown } = useCheckpoints();

  useEffect(() => {
    dispatch(registerVisibleChatMount({ chatId }));
    return () => {
      dispatch(unregisterVisibleChatMount({ chatId }));
    };
  }, [dispatch, chatId]);

  const preventSend = useAppSelector((state) =>
    selectPreventSendById(state, chatId),
  );
  const onEnableSend = () => dispatch(enableSend({ id: chatId }));

  const bottomDockRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const dock = bottomDockRef.current;
    // The chat root renders through DropzoneProvider's asChild cloneElement,
    // which overwrites any ref passed to it (react-dropzone's rootProps
    // carry their own ref) — so resolve the root as the dock's parent.
    const root = dock?.parentElement;
    if (!dock || !root) return;

    const updateClearance = () => {
      root.style.setProperty(
        "--rf-composer-clearance",
        `${dock.offsetHeight}px`,
      );
    };

    updateClearance();
    const observer = new ResizeObserver(updateClearance);
    observer.observe(dock);
    return () => observer.disconnect();
  }, []);

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

  return (
    <DropzoneProvider asChild>
      <Flex
        className={styles.chatRoot}
        style={{
          ...style,
          minHeight: 0,
          minWidth: 0,
          maxWidth: "100%",
          height: "100%",
          overflow: "hidden",
        }}
        direction="column"
        flexGrow="1"
        width="100%"
        px="1"
      >
        {isBrowserOpen && <BrowserPanel chatId={chatId} />}
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
          <ChatContent onRetry={handleRetry} onStopStreaming={handleAbort} />
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
    </DropzoneProvider>
  );
};
