import { useEffect, useRef, useReducer } from "react";
import { useAppDispatch } from "./useAppDispatch";
import { useAppSelector } from "./useAppSelector";
import { selectLspPort, selectApiKey } from "../features/Config/configSlice";
import {
  selectChatId,
  selectQueuedMessages,
  selectIsWaiting,
  selectIsStreaming,
  selectPreventSend,
  selectThreadPause,
  selectHasUncalledTools,
} from "../features/Chat/Thread/selectors";
import { dequeueUserMessage } from "../features/Chat/Thread";
import {
  sendUserMessage,
  type MessageContent,
} from "../services/refact/chatCommands";

export function useQueueAutoFlush() {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectChatId);
  const port = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);
  const queued = useAppSelector(selectQueuedMessages);
  const isWaiting = useAppSelector(selectIsWaiting);
  const isStreaming = useAppSelector(selectIsStreaming);
  const preventSend = useAppSelector(selectPreventSend);
  const paused = useAppSelector(selectThreadPause);
  const hasUncalledTools = useAppSelector(selectHasUncalledTools);

  const inFlightRef = useRef(false);
  const [retryTick, bumpRetry] = useReducer((x) => x + 1, 0);

  useEffect(() => {
    if (!chatId || !port) return;
    if (inFlightRef.current) return;
    if (queued.length === 0) return;
    if (isStreaming) return;
    if (paused) return;

    const next = queued[0];

    const canSendPriority = next.priority && !hasUncalledTools;
    const canSendRegular = !next.priority && !isWaiting && !preventSend;

    if (!canSendPriority && !canSendRegular) return;

    inFlightRef.current = true;

    void (async () => {
      try {
        await sendUserMessage(
          chatId,
          next.message.content as MessageContent,
          port,
          apiKey ?? undefined,
        );
        dispatch(dequeueUserMessage({ chatId, queuedId: next.id }));
      } catch {
        window.setTimeout(() => bumpRetry(), 2000);
      } finally {
        inFlightRef.current = false;
      }
    })();
  }, [
    chatId,
    port,
    apiKey,
    queued,
    isWaiting,
    isStreaming,
    paused,
    preventSend,
    hasUncalledTools,
    dispatch,
    retryTick,
  ]);
}
