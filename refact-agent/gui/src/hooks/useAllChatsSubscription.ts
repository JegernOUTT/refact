import { useEffect, useRef, useCallback, useMemo } from "react";
import { useAppDispatch } from "./useAppDispatch";
import { useAppSelector } from "./useAppSelector";
import {
  applyChatEvent,
  clearSseRefreshRequest,
} from "../features/Chat/Thread/actions";
import {
  selectOpenThreadIds,
  selectCurrentThreadId,
  selectSseRefreshRequested,
} from "../features/Chat/Thread/selectors";
import { selectLspPort, selectApiKey } from "../features/Config/configSlice";
import { subscribeToChatEvents } from "../services/refact/chatSubscription";
import {
  setSseStatus,
  sseEventReceived,
  removeSseConnection,
  clearAllSseConnections,
} from "../features/Connection";
import { calculateBackoff } from "../utils/backoff";

export function useAllChatsSubscription() {
  const dispatch = useAppDispatch();
  const port = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);
  const openThreadIds = useAppSelector(selectOpenThreadIds);
  const currentThreadId = useAppSelector(selectCurrentThreadId);
  const sseRefreshRequested = useAppSelector(selectSseRefreshRequested);

  const subscriptionsRef = useRef<Map<string, () => void>>(new Map());
  const seqMapRef = useRef<Map<string, bigint>>(new Map());
  const manualCloseRef = useRef<Set<string>>(new Set());
  const desiredIdsRef = useRef<Set<string>>(new Set());
  const retryCountRef = useRef<Map<string, number>>(new Map());
  const timeoutRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(
    new Map(),
  );
  const lastActivityDispatchRef = useRef<Map<string, number>>(new Map());
  const portRef = useRef(port);
  const apiKeyRef = useRef(apiKey);
  const subscribeRef = useRef<((chatId: string) => void) | null>(null);

  const ACTIVITY_THROTTLE_MS = 500;

  const allChatIds = useMemo(() => {
    const ids = new Set(openThreadIds);
    if (currentThreadId) {
      ids.add(currentThreadId);
    }
    return Array.from(ids);
  }, [openThreadIds, currentThreadId]);

  const clearPendingTimeout = useCallback((chatId: string) => {
    const existingTimeout = timeoutRef.current.get(chatId);
    if (existingTimeout) {
      clearTimeout(existingTimeout);
      timeoutRef.current.delete(chatId);
    }
  }, []);

  const scheduleResubscribe = useCallback(
    (chatId: string, useBackoff = false) => {
      clearPendingTimeout(chatId);

      const retryCount = retryCountRef.current.get(chatId) ?? 0;
      const delay = useBackoff ? calculateBackoff(retryCount) : 100;

      const timeoutId = setTimeout(() => {
        timeoutRef.current.delete(chatId);
        if (!desiredIdsRef.current.has(chatId)) return;
        if (subscriptionsRef.current.has(chatId)) return;
        subscribeRef.current?.(chatId);
      }, delay);

      timeoutRef.current.set(chatId, timeoutId);
    },
    [clearPendingTimeout],
  );

  const subscribeToChat = useCallback(
    (chatId: string) => {
      if (subscriptionsRef.current.has(chatId)) return;
      if (!portRef.current) return;
      if (!desiredIdsRef.current.has(chatId)) return;

      manualCloseRef.current.delete(chatId);
      seqMapRef.current.set(chatId, 0n);

      dispatch(setSseStatus({ chatId, status: "connecting" }));

      const unsubscribe = subscribeToChatEvents(
        chatId,
        portRef.current,
        {
          onEvent: (envelope) => {
            const seq = BigInt(envelope.seq);
            const lastSeq = seqMapRef.current.get(chatId) ?? 0n;

            if (envelope.type === "snapshot") {
              seqMapRef.current.set(chatId, seq);
              retryCountRef.current.set(chatId, 0);
              dispatch(setSseStatus({ chatId, status: "connected" }));
            } else {
              if (seq <= lastSeq) return;
              if (seq > lastSeq + 1n) {
                const unsub = subscriptionsRef.current.get(chatId);
                if (unsub) {
                  manualCloseRef.current.add(chatId);
                  unsub();
                  subscriptionsRef.current.delete(chatId);
                }
                dispatch(setSseStatus({ chatId, status: "connecting" }));
                scheduleResubscribe(chatId, false);
                return;
              }
              seqMapRef.current.set(chatId, seq);
            }
            dispatch(sseEventReceived({ chatId }));
            dispatch(applyChatEvent(envelope));
          },
          onConnected: () => {
            dispatch(setSseStatus({ chatId, status: "connecting" }));
          },
          onError: (error) => {
            subscriptionsRef.current.delete(chatId);
            const count = (retryCountRef.current.get(chatId) ?? 0) + 1;
            retryCountRef.current.set(chatId, count);
            dispatch(
              setSseStatus({
                chatId,
                status: "disconnected",
                error: error.message,
              }),
            );
            if (!manualCloseRef.current.has(chatId)) {
              scheduleResubscribe(chatId, true);
            }
          },
          onDisconnected: () => {
            subscriptionsRef.current.delete(chatId);
            const count = (retryCountRef.current.get(chatId) ?? 0) + 1;
            retryCountRef.current.set(chatId, count);
            dispatch(setSseStatus({ chatId, status: "disconnected" }));
            if (!manualCloseRef.current.has(chatId)) {
              scheduleResubscribe(chatId, true);
            }
          },
          onActivity: () => {
            const now = Date.now();
            const lastDispatch =
              lastActivityDispatchRef.current.get(chatId) ?? 0;
            if (now - lastDispatch >= ACTIVITY_THROTTLE_MS) {
              lastActivityDispatchRef.current.set(chatId, now);
              dispatch(sseEventReceived({ chatId }));
            }
          },
        },
        apiKeyRef.current ?? undefined,
      );

      subscriptionsRef.current.set(chatId, unsubscribe);
    },
    [dispatch, scheduleResubscribe],
  );

  subscribeRef.current = subscribeToChat;
  const subscribe = subscribeToChat;

  const unsubscribe = useCallback(
    (chatId: string) => {
      manualCloseRef.current.add(chatId);
      clearPendingTimeout(chatId);
      const unsub = subscriptionsRef.current.get(chatId);
      if (unsub) {
        unsub();
        subscriptionsRef.current.delete(chatId);
        seqMapRef.current.delete(chatId);
        retryCountRef.current.delete(chatId);
        lastActivityDispatchRef.current.delete(chatId);
        dispatch(removeSseConnection({ chatId }));
      }
    },
    [dispatch, clearPendingTimeout],
  );

  const unsubscribeAll = useCallback(() => {
    for (const chatId of subscriptionsRef.current.keys()) {
      manualCloseRef.current.add(chatId);
    }
    for (const unsub of subscriptionsRef.current.values()) {
      unsub();
    }
    for (const timeoutId of timeoutRef.current.values()) {
      clearTimeout(timeoutId);
    }
    subscriptionsRef.current.clear();
    seqMapRef.current.clear();
    retryCountRef.current.clear();
    timeoutRef.current.clear();
    lastActivityDispatchRef.current.clear();
    dispatch(clearAllSseConnections());
  }, [dispatch]);

  useEffect(() => {
    if (port !== portRef.current || apiKey !== apiKeyRef.current) {
      unsubscribeAll();
      portRef.current = port;
      apiKeyRef.current = apiKey;
    }

    if (!port) return;

    const currentIds = new Set(allChatIds);
    desiredIdsRef.current = currentIds;
    const subscribedIds = new Set(subscriptionsRef.current.keys());

    for (const id of currentIds) {
      if (!subscribedIds.has(id)) {
        subscribe(id);
      }
    }

    for (const id of subscribedIds) {
      if (!currentIds.has(id)) {
        unsubscribe(id);
      }
    }
  }, [allChatIds, port, apiKey, subscribe, unsubscribe, unsubscribeAll]);

  useEffect(() => {
    if (sseRefreshRequested) {
      dispatch(clearSseRefreshRequest());
      const chatId = sseRefreshRequested;
      unsubscribe(chatId);
      setTimeout(() => subscribe(chatId), 50);
    }
  }, [sseRefreshRequested, dispatch, subscribe, unsubscribe]);

  useEffect(() => {
    return () => {
      unsubscribeAll();
    };
  }, [unsubscribeAll]);

  useEffect(() => {
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        for (const chatId of desiredIdsRef.current) {
          if (!subscriptionsRef.current.has(chatId)) {
            retryCountRef.current.set(chatId, 0);
            subscribe(chatId);
          }
        }
      }
    };

    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [subscribe]);
}
