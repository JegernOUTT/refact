import { useCallback } from "react";
import { useAppSelector } from "./useAppSelector";
import { useAppDispatch } from "./useAppDispatch";
import { selectLspPort, selectApiKey } from "../features/Config/configSlice";
import {
  selectChatId,
  selectThreadImages,
  selectSendImmediately,
} from "../features/Chat/Thread/selectors";
import { resetThreadImages, setSendImmediately } from "../features/Chat/Thread";
import {
  sendUserMessage,
  retryFromIndex as retryFromIndexApi,
  regenerate as regenerateApi,
  updateChatParams,
  abortGeneration,
  respondToToolConfirmation,
  respondToToolConfirmations,
  updateMessage as updateMessageApi,
  removeMessage as removeMessageApi,
  cancelQueuedItem,
  type MessageContent,
} from "../services/refact/chatCommands";
import type { UserMessage } from "../services/refact/types";

type ContentItem =
  | { type: "text"; text: string }
  | { type: "image_url"; image_url: { url: string } };

function convertUserMessageContent(
  newContent: UserMessage["content"],
): MessageContent {
  if (typeof newContent === "string") {
    return newContent;
  }
  if (!Array.isArray(newContent)) {
    return "";
  }
  const mapped: ContentItem[] = [];
  for (const item of newContent) {
    if ("type" in item) {
      if (item.type === "text" && "text" in item) {
        mapped.push({ type: "text", text: item.text });
      } else if ("image_url" in item) {
        mapped.push({ type: "image_url", image_url: item.image_url });
      }
    } else if ("m_type" in item && "m_content" in item) {
      const { m_type, m_content } = item;
      if (m_type === "text") {
        mapped.push({ type: "text", text: String(m_content) });
      } else if (m_type.startsWith("image/")) {
        mapped.push({
          type: "image_url",
          image_url: { url: `data:${m_type};base64,${String(m_content)}` },
        });
      }
    }
  }
  return mapped.length > 0 ? mapped : "";
}

export function useChatActions() {
  const dispatch = useAppDispatch();
  const port = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);
  const chatId = useAppSelector(selectChatId);
  const attachedImages = useAppSelector(selectThreadImages);
  const sendImmediately = useAppSelector(selectSendImmediately);

  /**
   * Build message content with attached images if any.
   */
  const buildMessageContent = useCallback(
    (text: string): MessageContent => {
      if (attachedImages.length === 0) {
        return text;
      }

      const imageContents: { type: "image_url"; image_url: { url: string } }[] =
        [];
      for (const img of attachedImages) {
        if (typeof img.content === "string") {
          imageContents.push({
            type: "image_url",
            image_url: { url: img.content },
          });
        }
      }

      if (imageContents.length === 0) {
        return text;
      }

      if (text.trim().length === 0) {
        return imageContents;
      }

      return [{ type: "text" as const, text }, ...imageContents];
    },
    [attachedImages],
  );

  const submit = useCallback(
    async (question: string, priority?: boolean) => {
      if (!chatId || !port) return;

      const content = buildMessageContent(question);
      const isEmpty =
        typeof content === "string"
          ? content.trim().length === 0
          : content.length === 0;
      if (isEmpty) return;

      const shouldPrioritize = priority ?? sendImmediately;
      await sendUserMessage(
        chatId,
        content,
        port,
        apiKey ?? undefined,
        shouldPrioritize,
      );
      dispatch(resetThreadImages({ id: chatId }));
      dispatch(setSendImmediately(false));
    },
    [chatId, port, apiKey, buildMessageContent, dispatch, sendImmediately],
  );

  /**
   * Abort the current generation.
   */
  const abort = useCallback(async () => {
    if (!chatId || !port) return;
    await abortGeneration(chatId, port, apiKey ?? undefined);
  }, [chatId, port, apiKey]);

  /**
   * Update chat parameters (model, mode, etc.).
   */
  const setParams = useCallback(
    async (params: {
      model?: string;
      mode?: string;
      boost_reasoning?: boolean;
    }) => {
      if (!chatId || !port) return;
      await updateChatParams(chatId, params, port, apiKey ?? undefined);
    },
    [chatId, port, apiKey],
  );

  /**
   * Respond to tool confirmation (accept or reject).
   */
  const respondToTool = useCallback(
    async (toolCallId: string, accepted: boolean) => {
      if (!chatId || !port) return;
      await respondToToolConfirmation(
        chatId,
        toolCallId,
        accepted,
        port,
        apiKey ?? undefined,
      );
    },
    [chatId, port, apiKey],
  );

  /**
   * Respond to multiple tool confirmations at once (batch).
   */
  const respondToTools = useCallback(
    async (decisions: { tool_call_id: string; accepted: boolean }[]) => {
      if (!chatId || !port || decisions.length === 0) return;
      await respondToToolConfirmations(
        chatId,
        decisions,
        port,
        apiKey ?? undefined,
      );
    },
    [chatId, port, apiKey],
  );

  /**
   * Retry from a specific message index.
   * This truncates all messages from the given index and sends a new user message.
   */
  const retryFromIndex = useCallback(
    async (index: number, newContent: UserMessage["content"]) => {
      if (!chatId || !port) return;

      const content = convertUserMessageContent(newContent);

      await retryFromIndexApi(
        chatId,
        index,
        content,
        port,
        apiKey ?? undefined,
      );
    },
    [chatId, port, apiKey],
  );

  const updateMessage = useCallback(
    async (
      messageId: string,
      newContent: MessageContent,
      regenerate?: boolean,
    ) => {
      if (!chatId || !port) return;
      await updateMessageApi(
        chatId,
        messageId,
        newContent,
        port,
        apiKey ?? undefined,
        regenerate,
      );
    },
    [chatId, port, apiKey],
  );

  const removeMessage = useCallback(
    async (messageId: string, regenerate?: boolean) => {
      if (!chatId || !port) return;
      await removeMessageApi(
        chatId,
        messageId,
        port,
        apiKey ?? undefined,
        regenerate,
      );
    },
    [chatId, port, apiKey],
  );

  const regenerate = useCallback(async () => {
    if (!chatId || !port) return;
    await regenerateApi(chatId, port, apiKey ?? undefined);
  }, [chatId, port, apiKey]);

  const cancelQueued = useCallback(
    async (clientRequestId: string) => {
      if (!chatId || !port) return false;
      return cancelQueuedItem(
        chatId,
        clientRequestId,
        port,
        apiKey ?? undefined,
      );
    },
    [chatId, port, apiKey],
  );

  return {
    submit,
    abort,
    setParams,
    respondToTool,
    respondToTools,
    retryFromIndex,
    updateMessage,
    removeMessage,
    regenerate,
    cancelQueued,
  };
}

export default useChatActions;
