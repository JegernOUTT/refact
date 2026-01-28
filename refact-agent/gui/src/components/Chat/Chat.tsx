import React, { useCallback, useState } from "react";
import { ChatForm, ChatFormProps } from "../ChatForm";
import { ChatContent } from "../ChatContent";
import { Flex, Button, Text, Card } from "@radix-ui/themes";
import { useAppSelector, useAppDispatch, useChatActions } from "../../hooks";
import { type Config } from "../../features/Config/configSlice";
import {
  enableSend,
  selectIsStreaming,
  selectPreventSend,
  selectChatId,
  selectMessages,
  selectThreadMode,
} from "../../features/Chat/Thread";
import { ThreadHistoryButton } from "../Buttons";
import { push } from "../../features/Pages/pagesSlice";
import { DropzoneProvider } from "../Dropzone";
import { useCheckpoints } from "../../hooks/useCheckpoints";
import { Checkpoints } from "../../features/Checkpoints";
import { EnhancedModelSelector } from "./EnhancedModelSelector";

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
  const isStreaming = useAppSelector(selectIsStreaming);

  const chatId = useAppSelector(selectChatId);

  const { submit, abort, retryFromIndex } = useChatActions();

  const chatMode = useAppSelector(selectThreadMode);
  const messages = useAppSelector(selectMessages);

  const { shouldCheckpointsPopupBeShown } = useCheckpoints();

  const [isDebugChatHistoryVisible, setIsDebugChatHistoryVisible] =
    useState(false);

  const preventSend = useAppSelector(selectPreventSend);
  const onEnableSend = () => dispatch(enableSend({ id: chatId }));

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

  const handleThreadHistoryPage = useCallback(() => {
    dispatch(push({ name: "thread history page", chatId }));
  }, [chatId, dispatch]);

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
        style={{ ...style, minHeight: 0, height: "100%" }}
        direction="column"
        flexGrow="1"
        width="100%"
        px="1"
      >
        <Flex
          direction="column"
          style={{ flex: "1 1 auto", minHeight: 0, overflow: "hidden" }}
        >
          <ChatContent onRetry={handleRetry} onStopStreaming={handleAbort} />
        </Flex>

        <Flex direction="column" style={{ flex: "0 0 auto" }}>
          {shouldCheckpointsPopupBeShown && <Checkpoints />}

          {!isStreaming && preventSend && unCalledTools && (
            <Flex py="4">
              <Card style={{ width: "100%" }}>
                <Flex direction="column" align="center" gap="2" width="100%">
                  Chat was interrupted with uncalled tools calls.
                  <Button onClick={onEnableSend}>Resume</Button>
                </Flex>
              </Card>
            </Flex>
          )}

          <ChatForm
            key={chatId}
            onSubmit={handleSubmit}
            onClose={maybeSendToSidebar}
          />

          <Flex justify="between" pl="1" pr="1" pt="1">
            {messages.length > 0 && (
              <Flex align="center" justify="between" width="100%">
                <Flex align="center" gap="2">
                  <EnhancedModelSelector disabled={isStreaming} />
                  <Text size="1" color="gray">
                    •
                  </Text>
                  <Text
                    size="1"
                    color="gray"
                    onClick={() =>
                      setIsDebugChatHistoryVisible((prev) => !prev)
                    }
                    style={{ cursor: "pointer" }}
                  >
                    mode: {chatMode}
                  </Text>
                </Flex>
                {messages.length !== 0 &&
                  !isStreaming &&
                  isDebugChatHistoryVisible && (
                    <ThreadHistoryButton
                      title="View history of current thread"
                      size="1"
                      onClick={handleThreadHistoryPage}
                    />
                  )}
              </Flex>
            )}
          </Flex>
        </Flex>
      </Flex>
    </DropzoneProvider>
  );
};
