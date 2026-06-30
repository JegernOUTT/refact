import { useState, useEffect, useCallback, useRef } from "react";
import { useThreadId } from "../features/Chat/Thread";
import {
  saveDraftMessage,
  getDraftMessage,
  clearDraftMessage,
} from "../utils/threadStorage";
import { useDebounceCallback } from "usehooks-ts";

export function useDraftMessage() {
  const chatId = useThreadId();
  const [value, setValueInternal] = useState<string>(() => {
    if (chatId) {
      return getDraftMessage(chatId);
    }
    return "";
  });
  const prevChatIdRef = useRef<string>(chatId);
  const isInitialMount = useRef<boolean>(true);

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
      setValueInternal(draft);
      prevChatIdRef.current = chatId;
    }
  }, [chatId, debouncedSave]);

  const setValue = useCallback(
    (newValue: string | ((prev: string) => string)) => {
      setValueInternal((prev) => {
        const next = typeof newValue === "function" ? newValue(prev) : newValue;
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
      clearDraftMessage(chatId);
      setValueInternal("");
    }
  }, [chatId]);

  return { value, setValue, clearDraft };
}
