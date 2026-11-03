import { useState, useEffect, useCallback, useRef } from "react";
import { useThreadId } from "../features/Chat/Thread";
import {
  saveDraftMessage,
  getDraftMessage,
  clearDraftMessage,
} from "../utils/threadStorage";
import {
  QUEUED_MESSAGE_EDIT_EVENT,
  type QueuedMessageEditDetail,
} from "../utils/queuedMessageEdit";
import { useDebounceCallback } from "usehooks-ts";

export function useDraftMessage(
  onQueuedMessageEdit?: (previousValue: string) => void,
) {
  const chatId = useThreadId();
  const [value, setValueInternal] = useState<string>(() => {
    if (chatId) {
      return getDraftMessage(chatId);
    }
    return "";
  });
  const prevChatIdRef = useRef<string>(chatId);
  const isInitialMount = useRef<boolean>(true);
  const valueRef = useRef<string>(value);
  const onQueuedMessageEditRef = useRef(onQueuedMessageEdit);

  useEffect(() => {
    valueRef.current = value;
  }, [value]);

  useEffect(() => {
    onQueuedMessageEditRef.current = onQueuedMessageEdit;
  }, [onQueuedMessageEdit]);

  const debouncedSave = useDebounceCallback((id: string, content: string) => {
    saveDraftMessage(id, content);
  }, 500);

  useEffect(() => {
    return () => {
      debouncedSave.flush();
    };
  }, [debouncedSave]);

  useEffect(() => {
    if (isInitialMount.current) {
      isInitialMount.current = false;
      return;
    }

    if (chatId && chatId !== prevChatIdRef.current) {
      debouncedSave.flush();

      const draft = getDraftMessage(chatId);
      valueRef.current = draft;
      setValueInternal(draft);
      prevChatIdRef.current = chatId;
    }
  }, [chatId, debouncedSave]);

  useEffect(() => {
    if (!chatId) return;
    const handleQueuedMessageEdit = (event: Event) => {
      const detail = (event as CustomEvent<QueuedMessageEditDetail>).detail;
      if (detail.chatId !== chatId) return;
      const previous = valueRef.current;
      detail.composers.push({
        value: previous,
        apply: (next) => {
          debouncedSave.cancel();
          valueRef.current = next;
          setValueInternal(next);
          onQueuedMessageEditRef.current?.(previous);
        },
      });
    };
    window.addEventListener(QUEUED_MESSAGE_EDIT_EVENT, handleQueuedMessageEdit);
    return () =>
      window.removeEventListener(
        QUEUED_MESSAGE_EDIT_EVENT,
        handleQueuedMessageEdit,
      );
  }, [chatId, debouncedSave]);

  const setValue = useCallback(
    (newValue: string | ((prev: string) => string)) => {
      setValueInternal((prev) => {
        const next = typeof newValue === "function" ? newValue(prev) : newValue;
        valueRef.current = next;
        if (chatId) {
          debouncedSave(chatId, next);
        }
        return next;
      });
    },
    [chatId, debouncedSave],
  );

  const clearDraft = useCallback(() => {
    if (chatId) {
      debouncedSave.cancel();
      clearDraftMessage(chatId);
      valueRef.current = "";
      setValueInternal("");
    }
  }, [chatId, debouncedSave]);

  return { value, setValue, clearDraft };
}
