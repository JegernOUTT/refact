import { useEffect, useRef, useCallback } from "react";
import { useAppDispatch } from "./useAppDispatch";
import { useAppSelector } from "./useAppSelector";
import {
  applyChatEvent,
  clearSseRefreshRequest,
} from "../features/Chat/Thread/actions";
import {
  selectCurrentThreadId,
  selectOpenThreadIds,
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
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";

export function useAllChatsSubscription() {
  const dispatch = useAppDispatch();
  const port = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);
  const currentThreadId = useAppSelector(selectCurrentThreadId);
  const openThreadIds = useAppSelector(selectOpenThreadIds);
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
  const lastActivityAtRef = useRef<Map<string, number>>(new Map());
  const streamDeltaFlushRef = useRef<
    Map<string, ReturnType<typeof setTimeout>>
  >(new Map());
  const pendingStreamDeltaRef = useRef<
    Map<string, Extract<ChatEventEnvelope, { type: "stream_delta" }>>
  >(new Map());
  const streamedBytesRef = useRef<Map<string, number>>(new Map());
  const portRef = useRef(port);
  const apiKeyRef = useRef(apiKey);
  const subscribeRef = useRef<((chatId: string) => void) | null>(null);
  const enqueueStreamDeltaRef = useRef<
    | ((
        chatId: string,
        envelope: Extract<ChatEventEnvelope, { type: "stream_delta" }>,
      ) => void)
    | null
  >(null);
  const flushPendingStreamDeltaForChatRef = useRef<
    ((chatId: string) => void) | null
  >(null);
  const clearStreamDeltaFlushForChatRef = useRef<
    ((chatId: string) => void) | null
  >(null);

  const STALE_THRESHOLD_MS = 45_000;

  const ACTIVITY_THROTTLE_MS = 500;
  const MAX_MERGED_DELTA_OPS = 256;

  // Adaptive flush thresholds (bytes of accumulated content)
  const FLUSH_TIER_FAST_BYTES = 8_192;
  const FLUSH_TIER_MEDIUM_BYTES = 200_000;
  // Flush intervals per tier (ms)
  const FLUSH_MS_FAST = 0; // RAF (~16ms)
  const FLUSH_MS_MEDIUM = 150;
  const FLUSH_MS_SLOW = 500;
  const FLUSH_MS_BACKGROUND = 500;
  // Hard cap: force flush if buffered text exceeds this
  const MAX_BUFFERED_BYTES = 2_000_000;

  const activeChatId = currentThreadId;

  const clearPendingTimeout = useCallback((chatId: string) => {
    const existingTimeout = timeoutRef.current.get(chatId);
    if (existingTimeout) {
      clearTimeout(existingTimeout);
      timeoutRef.current.delete(chatId);
    }
  }, []);

  const clearStreamDeltaFlushForChat = useCallback((chatId: string) => {
    const timerId = streamDeltaFlushRef.current.get(chatId);
    if (timerId != null) {
      clearTimeout(timerId);
      streamDeltaFlushRef.current.delete(chatId);
    }
  }, []);

  const flushPendingStreamDeltaForChat = useCallback(
    (chatId: string) => {
      const pending = pendingStreamDeltaRef.current.get(chatId);
      if (!pending) return;
      pendingStreamDeltaRef.current.delete(chatId);
      dispatch(applyChatEvent(pending));
    },
    [dispatch],
  );

  const getFlushDelayMs = useCallback(
    (chatId: string): number => {
      const isActive = chatId === activeChatId;
      if (!isActive) return FLUSH_MS_BACKGROUND;
      const bytes = streamedBytesRef.current.get(chatId) ?? 0;
      if (bytes < FLUSH_TIER_FAST_BYTES) return FLUSH_MS_FAST;
      if (bytes < FLUSH_TIER_MEDIUM_BYTES) return FLUSH_MS_MEDIUM;
      return FLUSH_MS_SLOW;
    },
    [activeChatId],
  );

  const scheduleStreamDeltaFlushForChat = useCallback(
    (chatId: string) => {
      if (streamDeltaFlushRef.current.has(chatId)) return;

      const delayMs = getFlushDelayMs(chatId);

      const flush = () => {
        streamDeltaFlushRef.current.delete(chatId);
        flushPendingStreamDeltaForChat(chatId);
      };

      const id = setTimeout(flush, Math.max(delayMs, 0));
      streamDeltaFlushRef.current.set(chatId, id);
    },
    [flushPendingStreamDeltaForChat, getFlushDelayMs],
  );

  const enqueueStreamDelta = useCallback(
    (
      chatId: string,
      envelope: Extract<ChatEventEnvelope, { type: "stream_delta" }>,
    ) => {
      // Track accumulated content bytes for adaptive throttle
      for (const op of envelope.ops) {
        if (op.op === "append_content" || op.op === "append_reasoning") {
          const prev = streamedBytesRef.current.get(chatId) ?? 0;
          streamedBytesRef.current.set(chatId, prev + op.text.length);
        }
      }

      const pending = pendingStreamDeltaRef.current.get(chatId);
      if (pending && pending.message_id === envelope.message_id) {
        const mergedOpsLen = pending.ops.length + envelope.ops.length;
        if (mergedOpsLen <= MAX_MERGED_DELTA_OPS) {
          pending.seq = envelope.seq;
          pending.ops.push(...envelope.ops);
        } else {
          flushPendingStreamDeltaForChat(chatId);
          pendingStreamDeltaRef.current.set(chatId, envelope);
        }
      } else {
        flushPendingStreamDeltaForChat(chatId);
        pendingStreamDeltaRef.current.set(chatId, envelope);
      }

      // Force immediate flush if buffered text is too large
      const bufferedBytes = streamedBytesRef.current.get(chatId) ?? 0;
      if (bufferedBytes > MAX_BUFFERED_BYTES) {
        clearStreamDeltaFlushForChat(chatId);
        flushPendingStreamDeltaForChat(chatId);
        return;
      }

      scheduleStreamDeltaFlushForChat(chatId);
    },
    [
      flushPendingStreamDeltaForChat,
      scheduleStreamDeltaFlushForChat,
      clearStreamDeltaFlushForChat,
    ],
  );

  enqueueStreamDeltaRef.current = enqueueStreamDelta;
  flushPendingStreamDeltaForChatRef.current = flushPendingStreamDeltaForChat;
  clearStreamDeltaFlushForChatRef.current = clearStreamDeltaFlushForChat;

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
              flushPendingStreamDeltaForChatRef.current?.(chatId);
              streamedBytesRef.current.delete(chatId);
              seqMapRef.current.set(chatId, seq);
              retryCountRef.current.set(chatId, 0);
              dispatch(setSseStatus({ chatId, status: "connected" }));
            } else {
              if (seq <= lastSeq) return;
              if (seq > lastSeq + 1n) {
                flushPendingStreamDeltaForChatRef.current?.(chatId);
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
            if (envelope.type === "stream_delta") {
              enqueueStreamDeltaRef.current?.(chatId, envelope);
            } else {
              flushPendingStreamDeltaForChatRef.current?.(chatId);
              if (envelope.type === "stream_finished") {
                streamedBytesRef.current.delete(chatId);
              }
              dispatch(applyChatEvent(envelope));
            }
          },
          onConnected: () => {
            dispatch(setSseStatus({ chatId, status: "connected" }));
          },
          onError: (error) => {
            clearStreamDeltaFlushForChatRef.current?.(chatId);
            flushPendingStreamDeltaForChatRef.current?.(chatId);
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
            clearStreamDeltaFlushForChatRef.current?.(chatId);
            flushPendingStreamDeltaForChatRef.current?.(chatId);
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
            lastActivityAtRef.current.set(chatId, now);
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
      clearStreamDeltaFlushForChat(chatId);
      pendingStreamDeltaRef.current.delete(chatId);
      streamedBytesRef.current.delete(chatId);
      const unsub = subscriptionsRef.current.get(chatId);
      if (unsub) {
        unsub();
        subscriptionsRef.current.delete(chatId);
        seqMapRef.current.delete(chatId);
        retryCountRef.current.delete(chatId);
        lastActivityDispatchRef.current.delete(chatId);
        lastActivityAtRef.current.delete(chatId);
        dispatch(removeSseConnection({ chatId }));
      }
    },
    [dispatch, clearPendingTimeout, clearStreamDeltaFlushForChat],
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
    for (const flushId of streamDeltaFlushRef.current.values()) {
      clearTimeout(flushId);
    }
    subscriptionsRef.current.clear();
    seqMapRef.current.clear();
    manualCloseRef.current.clear();
    desiredIdsRef.current.clear();
    retryCountRef.current.clear();
    timeoutRef.current.clear();
    lastActivityDispatchRef.current.clear();
    lastActivityAtRef.current.clear();
    streamDeltaFlushRef.current.clear();
    pendingStreamDeltaRef.current.clear();
    streamedBytesRef.current.clear();
    dispatch(clearAllSseConnections());
  }, [dispatch]);

  useEffect(() => {
    if (port !== portRef.current || apiKey !== apiKeyRef.current) {
      unsubscribeAll();
      portRef.current = port;
      apiKeyRef.current = apiKey;
    }

    if (!port) return;

    const desired = new Set(openThreadIds);
    if (activeChatId) desired.add(activeChatId);
    desiredIdsRef.current = desired;
    const subscribedIds = Array.from(subscriptionsRef.current.keys());

    for (const id of subscribedIds) {
      if (!desiredIdsRef.current.has(id)) {
        unsubscribe(id);
      }
    }

    for (const id of desiredIdsRef.current) {
      if (!subscriptionsRef.current.has(id)) {
        subscribe(id);
      }
    }
  }, [
    activeChatId,
    openThreadIds,
    port,
    apiKey,
    subscribe,
    unsubscribe,
    unsubscribeAll,
  ]);

  useEffect(() => {
    if (!sseRefreshRequested) return;
    if (!portRef.current) return;

    dispatch(clearSseRefreshRequest());
    unsubscribe(sseRefreshRequested);
    setTimeout(() => subscribe(sseRefreshRequested), 50);
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
          const lastActivity = lastActivityAtRef.current.get(chatId) ?? 0;
          const isStale =
            lastActivity > 0 && Date.now() - lastActivity > STALE_THRESHOLD_MS;

          if (isStale && subscriptionsRef.current.has(chatId)) {
            retryCountRef.current.set(chatId, 0);
            unsubscribe(chatId);
            subscribe(chatId);
            continue;
          }

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
  }, [subscribe, unsubscribe]);
}
