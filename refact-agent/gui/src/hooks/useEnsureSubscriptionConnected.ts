import { useCallback, useRef } from "react";
import { useAppDispatch } from "./useAppDispatch";
import { useAppSelector } from "./useAppSelector";
import { selectSnapshotReceivedById } from "../features/Chat/Thread/selectors";
import { requestSseRefresh } from "../features/Chat/Thread/actions";
import { store } from "../app/store";

function waitUntil(
  predicate: () => boolean,
  timeoutMs: number,
  intervalMs: number,
): Promise<void> {
  return new Promise((resolve) => {
    if (predicate()) {
      resolve();
      return;
    }

    const start = Date.now();
    const interval = setInterval(() => {
      if (predicate() || Date.now() - start >= timeoutMs) {
        clearInterval(interval);
        resolve();
      }
    }, intervalMs);
  });
}

export function useEnsureSubscriptionConnected(
  chatId: string | null | undefined,
) {
  const dispatch = useAppDispatch();

  const chatIdRef = useRef(chatId);
  chatIdRef.current = chatId;

  const hasSnapshot = useCallback(() => {
    const id = chatIdRef.current;
    if (!id) return true;
    return selectSnapshotReceivedById(store.getState(), id);
  }, []);

  const hasCachedMessages = useCallback(() => {
    const id = chatIdRef.current;
    if (!id) return true;
    const rt = store.getState().chat.threads[id];
    return !!rt && rt.thread.messages.length > 0;
  }, []);

  const pendingRef = useRef<Promise<void> | null>(null);

  const ensureConnected = useCallback(async (): Promise<void> => {
    const targetChatId = chatIdRef.current;
    if (!targetChatId) return;

    if (hasSnapshot() || hasCachedMessages()) return;

    dispatch(requestSseRefresh({ chatId: targetChatId }));

    if (!pendingRef.current) {
      pendingRef.current = waitUntil(
        () =>
          chatIdRef.current !== targetChatId ||
          hasSnapshot() ||
          hasCachedMessages(),
        5000,
        100,
      ).finally(() => {
        pendingRef.current = null;
      });
    }

    await pendingRef.current;
  }, [dispatch, hasSnapshot, hasCachedMessages]);

  const isConnected = useAppSelector((state) => {
    if (!chatId) return true;
    return selectSnapshotReceivedById(state, chatId);
  });

  return {
    ensureConnected,
    isConnected,
    isConnecting: !isConnected,
  };
}
