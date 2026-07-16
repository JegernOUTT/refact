import { useEffect, useMemo, useRef, useCallback, useState } from "react";
import { shallowEqual } from "react-redux";
import { useAppDispatch } from "./useAppDispatch";
import { useAppSelector } from "./useAppSelector";
import {
  applyChatEvent,
  clearSseRefreshRequest,
  markThreadSseError,
} from "../features/Chat/Thread/actions";
import {
  selectCurrentThreadId,
  selectSseRefreshRequested,
} from "../features/Chat/Thread/selectors";
import { selectConfig } from "../features/Config/configSlice";
import {
  getEngineEndpointIdentity,
  hasUsableEngineEndpoint,
  type EngineApiConfig,
} from "../services/refact/apiUrl";
import { subscribeToChatEvents } from "../services/refact/chatSubscription";
import { invalidateChatParamsSyncState } from "../services/refact/chatCommands";
import {
  setSseStatus,
  sseEventReceived,
  removeSseConnection,
  clearAllSseConnections,
  selectVisibleChatMountIds,
  setConnectionSuspended,
} from "../features/Connection";
import { calculateBackoff } from "../utils/backoff";
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";
import { processCompleted } from "../features/Notifications";

type FlushHandle =
  | { type: "timeout"; id: ReturnType<typeof setTimeout> }
  | { type: "raf"; id: number };

function requestNextFrame(cb: () => void): FlushHandle | null {
  if (typeof globalThis.requestAnimationFrame !== "function") return null;
  return {
    type: "raf",
    id: globalThis.requestAnimationFrame(() => cb()),
  };
}

function cancelScheduledFlush(handle: FlushHandle) {
  if (handle.type === "raf") {
    if (typeof globalThis.cancelAnimationFrame === "function") {
      globalThis.cancelAnimationFrame(handle.id);
    }
    return;
  }
  clearTimeout(handle.id);
}

function isDocumentVisible(): boolean {
  if (typeof document === "undefined") return true;
  return document.visibilityState !== "hidden";
}

type PickDesiredChatSubscriptionsArgs = {
  visibleThreadIds?: string[];
  documentVisible?: boolean;
};

export function pickDesiredChatSubscriptions({
  visibleThreadIds = [],
  documentVisible = true,
}: PickDesiredChatSubscriptionsArgs): string[] {
  if (!documentVisible) return [];

  const ordered: string[] = [];
  const seen = new Set<string>();
  const push = (id: string | null | undefined) => {
    if (!id || seen.has(id)) return;
    seen.add(id);
    ordered.push(id);
  };

  for (const id of visibleThreadIds) {
    push(id);
  }

  return ordered;
}

export function useAllChatsSubscription() {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const endpointIdentity = getEngineEndpointIdentity(config);
  const hasEndpoint = hasUsableEngineEndpoint(config);
  const subscriptionConfig = useMemo(
    () => ({
      host: config.host,
      lspPort: config.lspPort,
      lspUrl: config.lspUrl,
      dev: config.dev,
      engineServed: config.engineServed,
    }),
    [
      config.host,
      config.lspPort,
      config.lspUrl,
      config.dev,
      config.engineServed,
    ],
  );
  const currentThreadId = useAppSelector(selectCurrentThreadId);
  const visibleThreadIds = useAppSelector(
    selectVisibleChatMountIds,
    shallowEqual,
  );
  const sseRefreshRequested = useAppSelector(selectSseRefreshRequested);
  const [documentVisible, setDocumentVisible] = useState(isDocumentVisible);

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
  const streamDeltaFlushRef = useRef<Map<string, FlushHandle>>(new Map());
  const pendingStreamDeltaRef = useRef<
    Map<string, Extract<ChatEventEnvelope, { type: "stream_delta" }>>
  >(new Map());
  const subchatFlushRef = useRef<Map<string, FlushHandle>>(new Map());
  const pendingSubchatUpdateRef = useRef<
    Map<string, Extract<ChatEventEnvelope, { type: "subchat_update" }>>
  >(new Map());
  const sseRefreshTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const streamedBytesRef = useRef<Map<string, number>>(new Map());
  const pendingBytesRef = useRef<Map<string, number>>(new Map());
  const configRef = useRef<EngineApiConfig>(subscriptionConfig);
  const endpointIdentityRef = useRef(endpointIdentity);
  const hasEndpointRef = useRef(hasEndpoint);
  const apiKeyRef = useRef(config.apiKey);
  const subscribeRef = useRef<((chatId: string) => void) | null>(null);
  const unsubscribeRef = useRef<((chatId: string) => void) | null>(null);
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

  const ACTIVITY_THROTTLE_MS = 500;
  const MAX_MERGED_DELTA_OPS = 256;

  // Adaptive flush thresholds (JS string length units, i.e. UTF-16 code units)
  const FLUSH_TIER_FAST_BYTES = 8_192;
  const FLUSH_TIER_MEDIUM_BYTES = 200_000;
  // Flush intervals per tier (ms)
  const FLUSH_MS_FAST = 50;
  const FLUSH_MS_MEDIUM = 250;
  const FLUSH_MS_SLOW = 750;
  const FLUSH_MS_BACKGROUND = 750;
  const SUBCHAT_FLUSH_MS_ACTIVE = 150;
  const SUBCHAT_FLUSH_MS_BACKGROUND = 750;
  // Hard cap: force flush if buffered char-count (UTF-16 units) exceeds this
  const MAX_BUFFERED_BYTES = 2_000_000;

  const activeChatId = currentThreadId;

  const clearPendingTimeout = useCallback((chatId: string) => {
    const existingTimeout = timeoutRef.current.get(chatId);
    if (existingTimeout) {
      clearTimeout(existingTimeout);
      timeoutRef.current.delete(chatId);
    }
  }, []);

  const clearRetryStateForUndesiredChats = useCallback(
    (desired: Set<string>) => {
      for (const chatId of Array.from(timeoutRef.current.keys())) {
        if (desired.has(chatId)) continue;
        clearPendingTimeout(chatId);
      }
      for (const chatId of Array.from(retryCountRef.current.keys())) {
        if (desired.has(chatId)) continue;
        retryCountRef.current.delete(chatId);
      }
    },
    [clearPendingTimeout],
  );

  // Clear all per-chat streaming state. Used by unsubscribe() and the
  // onError/onDisconnected callbacks so state never leaks between reconnects.
  const clearChatStreamState = useCallback((chatId: string) => {
    streamedBytesRef.current.delete(chatId);
    pendingBytesRef.current.delete(chatId);
    seqMapRef.current.delete(chatId);
    lastActivityAtRef.current.delete(chatId);
    lastActivityDispatchRef.current.delete(chatId);
  }, []);

  const clearStreamDeltaFlushForChat = useCallback((chatId: string) => {
    const handle = streamDeltaFlushRef.current.get(chatId);
    if (handle != null) {
      cancelScheduledFlush(handle);
      streamDeltaFlushRef.current.delete(chatId);
    }
  }, []);

  const clearSubchatFlushForChat = useCallback((chatId: string) => {
    const handle = subchatFlushRef.current.get(chatId);
    if (handle != null) {
      cancelScheduledFlush(handle);
      subchatFlushRef.current.delete(chatId);
    }
  }, []);

  const flushPendingStreamDeltaForChat = useCallback(
    (chatId: string) => {
      const pending = pendingStreamDeltaRef.current.get(chatId);
      if (!pending) return;
      pendingStreamDeltaRef.current.delete(chatId);
      pendingBytesRef.current.delete(chatId);
      dispatch(applyChatEvent(pending));
    },
    [dispatch],
  );

  const flushPendingSubchatUpdateForChat = useCallback(
    (chatId: string) => {
      const pending = pendingSubchatUpdateRef.current.get(chatId);
      if (!pending) return;
      pendingSubchatUpdateRef.current.delete(chatId);
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

      if (delayMs <= 0) {
        const frameHandle = requestNextFrame(flush);
        if (frameHandle) {
          streamDeltaFlushRef.current.set(chatId, frameHandle);
          return;
        }
      }

      streamDeltaFlushRef.current.set(chatId, {
        type: "timeout",
        id: setTimeout(flush, Math.max(delayMs, 0)),
      });
    },
    [flushPendingStreamDeltaForChat, getFlushDelayMs],
  );

  const scheduleSubchatFlushForChat = useCallback(
    (chatId: string) => {
      if (subchatFlushRef.current.has(chatId)) return;

      const flush = () => {
        subchatFlushRef.current.delete(chatId);
        flushPendingSubchatUpdateForChat(chatId);
      };
      const delayMs =
        chatId === activeChatId
          ? SUBCHAT_FLUSH_MS_ACTIVE
          : SUBCHAT_FLUSH_MS_BACKGROUND;

      subchatFlushRef.current.set(chatId, {
        type: "timeout",
        id: setTimeout(flush, delayMs),
      });
    },
    [activeChatId, flushPendingSubchatUpdateForChat],
  );

  const enqueueStreamDelta = useCallback(
    (
      chatId: string,
      envelope: Extract<ChatEventEnvelope, { type: "stream_delta" }>,
    ) => {
      // streamedCharsRef: total chars seen in this stream (never decrements),
      // used only for adaptive flush-tier selection.
      // pendingCharsRef: chars currently sitting in the pending buffer,
      // updated precisely after merge/replace — used for the force-flush cap.
      let deltaTextLen = 0;
      for (const op of envelope.ops) {
        if (
          op.op === "append_content" ||
          op.op === "append_reasoning" ||
          op.op === "set_reasoning"
        ) {
          deltaTextLen += op.text.length;
        }
      }
      streamedBytesRef.current.set(
        chatId,
        (streamedBytesRef.current.get(chatId) ?? 0) + deltaTextLen,
      );

      const pending = pendingStreamDeltaRef.current.get(chatId);
      if (pending && pending.message_id === envelope.message_id) {
        const mergedOpsLen = pending.ops.length + envelope.ops.length;
        if (mergedOpsLen <= MAX_MERGED_DELTA_OPS) {
          // Merging: add incoming chars to existing pending buffer
          pending.seq = envelope.seq;
          pending.ops.push(...envelope.ops);
          pendingBytesRef.current.set(
            chatId,
            (pendingBytesRef.current.get(chatId) ?? 0) + deltaTextLen,
          );
        } else {
          // Too many ops: flush existing, start fresh with incoming envelope
          flushPendingStreamDeltaForChat(chatId); // resets pendingBytesRef
          pendingStreamDeltaRef.current.set(chatId, envelope);
          pendingBytesRef.current.set(chatId, deltaTextLen);
        }
      } else {
        // Different message or no pending: flush existing, start with incoming
        flushPendingStreamDeltaForChat(chatId); // resets pendingBytesRef
        pendingStreamDeltaRef.current.set(chatId, envelope);
        pendingBytesRef.current.set(chatId, deltaTextLen);
      }

      // Force immediate flush if *buffered* (not total) chars exceed the cap
      const bufferedChars = pendingBytesRef.current.get(chatId) ?? 0;
      if (bufferedChars > MAX_BUFFERED_BYTES) {
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

  const enqueueSubchatUpdate = useCallback(
    (
      chatId: string,
      envelope: Extract<ChatEventEnvelope, { type: "subchat_update" }>,
    ) => {
      const pending = pendingSubchatUpdateRef.current.get(chatId);
      if (
        pending &&
        pending.tool_call_id === envelope.tool_call_id &&
        pending.chat_id === envelope.chat_id
      ) {
        pendingSubchatUpdateRef.current.set(chatId, {
          ...pending,
          seq: envelope.seq,
          subchat_id: envelope.subchat_id,
          attached_files: envelope.attached_files ?? pending.attached_files,
        });
      } else {
        flushPendingSubchatUpdateForChat(chatId);
        pendingSubchatUpdateRef.current.set(chatId, envelope);
      }

      scheduleSubchatFlushForChat(chatId);
    },
    [flushPendingSubchatUpdateForChat, scheduleSubchatFlushForChat],
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
      if (!hasEndpointRef.current) return;
      if (!desiredIdsRef.current.has(chatId)) return;

      manualCloseRef.current.delete(chatId);
      seqMapRef.current.set(chatId, 0n);

      dispatch(setSseStatus({ chatId, status: "connecting" }));

      const unsubscribe = subscribeToChatEvents(
        chatId,
        configRef.current,
        {
          onEvent: (envelope) => {
            const seq = BigInt(envelope.seq);
            const lastSeq = seqMapRef.current.get(chatId) ?? 0n;

            if (envelope.type === "snapshot") {
              flushPendingStreamDeltaForChatRef.current?.(chatId);
              streamedBytesRef.current.delete(chatId);
              pendingBytesRef.current.delete(chatId);
              seqMapRef.current.set(chatId, seq);
              retryCountRef.current.set(chatId, 0);
              dispatch(setSseStatus({ chatId, status: "connected" }));
            } else {
              if (seq <= lastSeq) return;
              if (seq > lastSeq + 1n) {
                flushPendingStreamDeltaForChatRef.current?.(chatId);
                unsubscribeRef.current?.(chatId);
                dispatch(setSseStatus({ chatId, status: "connecting" }));
                scheduleResubscribe(chatId, false);
                return;
              }
              seqMapRef.current.set(chatId, seq);
            }
            if (envelope.type === "stream_delta") {
              enqueueStreamDeltaRef.current?.(chatId, envelope);
            } else if (envelope.type === "subchat_update") {
              flushPendingStreamDeltaForChatRef.current?.(chatId);
              enqueueSubchatUpdate(chatId, envelope);
            } else {
              clearSubchatFlushForChat(chatId);
              flushPendingSubchatUpdateForChat(chatId);
              flushPendingStreamDeltaForChatRef.current?.(chatId);
              if (envelope.type === "stream_finished") {
                streamedBytesRef.current.delete(chatId);
                pendingBytesRef.current.delete(chatId);
              }
              if (envelope.type === "process_completed") {
                dispatch(processCompleted(envelope));
              }
              dispatch(applyChatEvent(envelope));
            }
          },
          onConnected: () => {
            dispatch(setSseStatus({ chatId, status: "connected" }));
          },
          onError: (error) => {
            invalidateChatParamsSyncState(chatId, configRef.current);
            clearStreamDeltaFlushForChatRef.current?.(chatId);
            flushPendingStreamDeltaForChatRef.current?.(chatId);
            clearSubchatFlushForChat(chatId);
            flushPendingSubchatUpdateForChat(chatId);
            dispatch(markThreadSseError({ id: chatId, error: error.message }));
            subscriptionsRef.current.delete(chatId);
            clearChatStreamState(chatId);
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
            invalidateChatParamsSyncState(chatId, configRef.current);
            clearStreamDeltaFlushForChatRef.current?.(chatId);
            flushPendingStreamDeltaForChatRef.current?.(chatId);
            clearSubchatFlushForChat(chatId);
            flushPendingSubchatUpdateForChat(chatId);
            subscriptionsRef.current.delete(chatId);
            clearChatStreamState(chatId);
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
    [
      clearChatStreamState,
      clearSubchatFlushForChat,
      dispatch,
      enqueueSubchatUpdate,
      flushPendingSubchatUpdateForChat,
      scheduleResubscribe,
    ],
  );

  subscribeRef.current = subscribeToChat;
  const subscribe = subscribeToChat;

  const unsubscribe = useCallback(
    (chatId: string) => {
      manualCloseRef.current.add(chatId);
      clearPendingTimeout(chatId);
      clearStreamDeltaFlushForChat(chatId);
      clearSubchatFlushForChat(chatId);
      pendingStreamDeltaRef.current.delete(chatId);
      pendingSubchatUpdateRef.current.delete(chatId);
      clearChatStreamState(chatId);
      const unsub = subscriptionsRef.current.get(chatId);
      if (unsub) {
        unsub();
        subscriptionsRef.current.delete(chatId);
      }
      retryCountRef.current.delete(chatId);
      dispatch(removeSseConnection({ chatId }));
    },
    [
      dispatch,
      clearPendingTimeout,
      clearSubchatFlushForChat,
      clearStreamDeltaFlushForChat,
      clearChatStreamState,
    ],
  );

  unsubscribeRef.current = unsubscribe;

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
    for (const flushHandle of streamDeltaFlushRef.current.values()) {
      cancelScheduledFlush(flushHandle);
    }
    for (const flushHandle of subchatFlushRef.current.values()) {
      cancelScheduledFlush(flushHandle);
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
    subchatFlushRef.current.clear();
    pendingSubchatUpdateRef.current.clear();
    streamedBytesRef.current.clear();
    pendingBytesRef.current.clear();
    dispatch(clearAllSseConnections());
  }, [dispatch]);

  useEffect(() => {
    if (
      endpointIdentity !== endpointIdentityRef.current ||
      config.apiKey !== apiKeyRef.current
    ) {
      unsubscribeAll();
      endpointIdentityRef.current = endpointIdentity;
      apiKeyRef.current = config.apiKey;
    }
    configRef.current = subscriptionConfig;
    hasEndpointRef.current = hasEndpoint;

    if (!hasEndpoint) {
      const desired = new Set<string>();
      desiredIdsRef.current = desired;
      clearRetryStateForUndesiredChats(desired);
      for (const chatId of Array.from(subscriptionsRef.current.keys())) {
        unsubscribe(chatId);
      }
      return;
    }

    const subscribedIds = Array.from(subscriptionsRef.current.keys());
    const desiredOrder = pickDesiredChatSubscriptions({
      visibleThreadIds,
      documentVisible,
    });
    const desired = new Set(desiredOrder);
    desiredIdsRef.current = desired;
    clearRetryStateForUndesiredChats(desired);

    for (const id of subscribedIds) {
      if (!desiredIdsRef.current.has(id)) {
        unsubscribe(id);
      }
    }

    for (const id of desiredOrder) {
      if (!subscriptionsRef.current.has(id)) {
        subscribe(id);
      }
    }
  }, [
    visibleThreadIds,
    documentVisible,
    config.apiKey,
    hasEndpoint,
    config.lspPort,
    endpointIdentity,
    clearRetryStateForUndesiredChats,
    subscribe,
    subscriptionConfig,
    unsubscribe,
    unsubscribeAll,
  ]);

  useEffect(() => {
    if (!sseRefreshRequested) return;
    if (!hasEndpoint) return;
    if (!desiredIdsRef.current.has(sseRefreshRequested)) return;
    if (sseRefreshTimeoutRef.current) {
      clearTimeout(sseRefreshTimeoutRef.current);
      sseRefreshTimeoutRef.current = null;
    }

    unsubscribe(sseRefreshRequested);
    sseRefreshTimeoutRef.current = setTimeout(() => {
      sseRefreshTimeoutRef.current = null;
      if (desiredIdsRef.current.has(sseRefreshRequested)) {
        subscribe(sseRefreshRequested);
      }
      dispatch(clearSseRefreshRequest());
    }, 50);
  }, [
    dispatch,
    endpointIdentity,
    hasEndpoint,
    sseRefreshRequested,
    subscribe,
    unsubscribe,
    visibleThreadIds,
    documentVisible,
  ]);

  useEffect(() => {
    return () => {
      if (sseRefreshTimeoutRef.current) {
        clearTimeout(sseRefreshTimeoutRef.current);
        sseRefreshTimeoutRef.current = null;
      }
      unsubscribeAll();
    };
  }, [unsubscribeAll]);

  useEffect(() => {
    if (
      typeof document === "undefined" ||
      typeof document.addEventListener !== "function"
    ) {
      return;
    }

    dispatch(setConnectionSuspended(!isDocumentVisible()));

    const handleVisibilityChange = () => {
      const nextDocumentVisible = isDocumentVisible();
      setDocumentVisible(nextDocumentVisible);
      dispatch(setConnectionSuspended(!nextDocumentVisible));

      if (nextDocumentVisible) return;

      desiredIdsRef.current = new Set();
      for (const chatId of Array.from(subscriptionsRef.current.keys())) {
        unsubscribe(chatId);
      }
    };

    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [dispatch, unsubscribe]);
}
