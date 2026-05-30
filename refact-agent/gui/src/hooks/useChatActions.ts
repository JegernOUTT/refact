import { useCallback } from "react";
import { useAppSelector } from "./useAppSelector";
import { useAppDispatch } from "./useAppDispatch";
import { selectConfig, selectApiKey } from "../features/Config/configSlice";
import {
  selectChatId,
  selectThread,
  selectThreadImages,
  selectSendImmediately,
  selectMessages,
  selectManualPreviewItems,
  selectManualPreviewRan,
} from "../features/Chat/Thread/selectors";
import {
  resetThreadImages,
  setSendImmediately,
  clearManualPreviewItems,
} from "../features/Chat/Thread";
import { buildThreadParamsPatch } from "../features/Chat/Thread/actions";
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
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);
  const chatId = useAppSelector(selectChatId);
  const thread = useAppSelector(selectThread);
  const attachedImages = useAppSelector(selectThreadImages);
  const sendImmediately = useAppSelector(selectSendImmediately);
  const messages = useAppSelector(selectMessages);
  const manualPreviewItems = useAppSelector(selectManualPreviewItems);
  const manualPreviewRan = useAppSelector(selectManualPreviewRan);

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
      if (!chatId) return;

      const content = buildMessageContent(question);
      const isEmpty =
        typeof content === "string"
          ? content.trim().length === 0
          : content.length === 0;
      if (isEmpty) return;

      if (thread) {
        const patch = buildThreadParamsPatch(thread, messages.length === 0);
        if (Object.keys(patch).length > 0) {
          await updateChatParams(chatId, patch, config, apiKey ?? undefined);
        }
      }

      const contextFiles =
        manualPreviewItems.length > 0
          ? manualPreviewItems.map((item) => item.context_file)
          : undefined;
      const shouldSuppressAutoEnrichment =
        manualPreviewRan &&
        contextFiles !== undefined &&
        contextFiles.length > 0;

      const shouldPrioritize = priority ?? sendImmediately;
      await sendUserMessage(
        chatId,
        content,
        config,
        apiKey ?? undefined,
        shouldPrioritize,
        contextFiles,
        shouldSuppressAutoEnrichment,
      );

      dispatch(clearManualPreviewItems({ chatId }));

      dispatch(resetThreadImages({ id: chatId }));
      dispatch(setSendImmediately(false));
    },
    [
      chatId,
      config,
      apiKey,
      buildMessageContent,
      dispatch,
      sendImmediately,
      messages,
      thread,
      manualPreviewItems,
      manualPreviewRan,
    ],
  );

  /**
   * Abort the current generation.
   */
  const abort = useCallback(async () => {
    if (!chatId) return;
    await abortGeneration(chatId, config, apiKey ?? undefined);
  }, [chatId, config, apiKey]);

  /**
   * Update chat parameters (model, mode, etc.).
   */
  const setParams = useCallback(
    async (params: {
      model?: string;
      mode?: string;
      boost_reasoning?: boolean;
    }) => {
      if (!chatId) return;
      await updateChatParams(chatId, params, config, apiKey ?? undefined);
    },
    [chatId, config, apiKey],
  );

  /**
   * Respond to tool confirmation (accept or reject).
   */
  const respondToTool = useCallback(
    async (toolCallId: string, accepted: boolean) => {
      if (!chatId) return;
      await respondToToolConfirmation(
        chatId,
        toolCallId,
        accepted,
        config,
        apiKey ?? undefined,
      );
    },
    [chatId, config, apiKey],
  );

  /**
   * Respond to multiple tool confirmations at once (batch).
   */
  const respondToTools = useCallback(
    async (decisions: { tool_call_id: string; accepted: boolean }[]) => {
      if (!chatId || decisions.length === 0) return;
      await respondToToolConfirmations(
        chatId,
        decisions,
        config,
        apiKey ?? undefined,
      );
    },
    [chatId, config, apiKey],
  );

  /**
   * Retry from a specific message index.
   * This truncates all messages from the given index and sends a new user message.
   */
  const retryFromIndex = useCallback(
    async (index: number, newContent: UserMessage["content"]) => {
      if (!chatId) return;

      const content = convertUserMessageContent(newContent);

      await retryFromIndexApi(
        chatId,
        index,
        content,
        config,
        apiKey ?? undefined,
      );
    },
    [chatId, config, apiKey],
  );

  const updateMessage = useCallback(
    async (
      messageId: string,
      newContent: MessageContent,
      regenerate?: boolean,
    ) => {
      if (!chatId) return;
      await updateMessageApi(
        chatId,
        messageId,
        newContent,
        config,
        apiKey ?? undefined,
        regenerate,
      );
    },
    [chatId, config, apiKey],
  );

  const removeMessage = useCallback(
    async (messageId: string, regenerate?: boolean) => {
      if (!chatId) return;
      await removeMessageApi(
        chatId,
        messageId,
        config,
        apiKey ?? undefined,
        regenerate,
      );
    },
    [chatId, config, apiKey],
  );

  const regenerate = useCallback(async () => {
    if (!chatId) return;
    await regenerateApi(chatId, config, apiKey ?? undefined);
  }, [chatId, config, apiKey]);

  const cancelQueued = useCallback(
    async (clientRequestId: string) => {
      if (!chatId) return false;
      return cancelQueuedItem(
        chatId,
        clientRequestId,
        config,
        apiKey ?? undefined,
      );
    },
    [chatId, config, apiKey],
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
