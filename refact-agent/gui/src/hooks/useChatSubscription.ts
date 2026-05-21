import { useCallback, useEffect, useRef, useState } from "react";
import { useAppDispatch } from "./useAppDispatch";
import { useAppSelector } from "./useAppSelector";
import {
  applyChatEvent,
  clearSseRefreshRequest,
  markThreadSseError,
} from "../features/Chat/Thread/actions";
import { selectSseRefreshRequested } from "../features/Chat/Thread/selectors";
import { selectLspPort, selectApiKey } from "../features/Config/configSlice";
import {
  subscribeToChatEvents,
  type ChatEventEnvelope,
} from "../services/refact/chatSubscription";

const DEBUG =
  typeof window !== "undefined" &&
  new URLSearchParams(window.location.search).has("debug");

export type ConnectionStatus = "disconnected" | "connecting" | "connected";

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

export type UseChatSubscriptionOptions = {
  /** Enable subscription (default: true) */
  enabled?: boolean;
  /** Reconnect on error (default: true) */
  autoReconnect?: boolean;
  /** Reconnect delay in ms (default: 2000) */
  reconnectDelay?: number;
  /** Callback when event received */
  onEvent?: (event: ChatEventEnvelope) => void;
  /** Callback when connected */
  onConnected?: () => void;
  /** Callback when disconnected */
  onDisconnected?: () => void;
  /** Callback when error occurs */
  onError?: (error: Error) => void;
};

/**
 * Hook for subscribing to chat events via SSE.
 *
 * @param chatId - Chat ID to subscribe to
 * @param options - Configuration options
 * @returns Connection status and control functions
 */
export function useChatSubscription(
  chatId: string | null | undefined,
  options: UseChatSubscriptionOptions = {},
) {
  const {
    enabled = true,
    autoReconnect = true,
    reconnectDelay = 2000,
    onEvent,
    onConnected,
    onDisconnected,
    onError,
  } = options;

  const dispatch = useAppDispatch();
  const port = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);

  const [status, setStatus] = useState<ConnectionStatus>("disconnected");
  const [error, setError] = useState<Error | null>(null);

  const lastSeqRef = useRef<bigint>(0n);
  const lastActivityAtRef = useRef<number>(0);
  const callbacksRef = useRef({
    onEvent,
    onConnected,
    onDisconnected,
    onError,
  });
  callbacksRef.current = { onEvent, onConnected, onDisconnected, onError };

  const STALE_THRESHOLD_MS = 45_000;

  const unsubscribeRef = useRef<(() => void) | null>(null);
  const reconnectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const streamDeltaFlushRef = useRef<FlushHandle | null>(null);
  const pendingStreamDeltaRef = useRef<Extract<
    ChatEventEnvelope,
    { type: "stream_delta" }
  > | null>(null);
  const subchatFlushRef = useRef<FlushHandle | null>(null);
  const pendingSubchatUpdateRef = useRef<Extract<
    ChatEventEnvelope,
    { type: "subchat_update" }
  > | null>(null);
  const streamedBytesRef = useRef(0);
  const pendingBytesRef = useRef(0);
  const connectingRef = useRef(false);
  // eslint-disable-next-line @typescript-eslint/no-empty-function
  const connectRef = useRef<() => void>(() => {});

  const MAX_MERGED_DELTA_OPS = 256;

  // Adaptive flush thresholds (JS string length units, i.e. UTF-16 code units)
  const FLUSH_TIER_FAST_BYTES = 8_192;
  const FLUSH_TIER_MEDIUM_BYTES = 200_000;
  const FLUSH_MS_FAST = 50;
  const FLUSH_MS_MEDIUM = 250;
  const FLUSH_MS_SLOW = 750;
  const SUBCHAT_FLUSH_MS = 150;
  // Hard cap: force flush if buffered char-count (UTF-16 units) exceeds this
  const MAX_BUFFERED_BYTES = 2_000_000;

  const clearStreamDeltaFlush = useCallback(() => {
    const handle = streamDeltaFlushRef.current;
    if (handle != null) {
      cancelScheduledFlush(handle);
      streamDeltaFlushRef.current = null;
    }
  }, []);

  const clearSubchatFlush = useCallback(() => {
    const handle = subchatFlushRef.current;
    if (handle != null) {
      cancelScheduledFlush(handle);
      subchatFlushRef.current = null;
    }
  }, []);

  const flushPendingStreamDelta = useCallback(() => {
    const pending = pendingStreamDeltaRef.current;
    if (!pending) return;
    pendingStreamDeltaRef.current = null;
    pendingBytesRef.current = 0;
    dispatch(applyChatEvent(pending));
    callbacksRef.current.onEvent?.(pending);
  }, [dispatch]);

  const flushPendingSubchatUpdate = useCallback(() => {
    const pending = pendingSubchatUpdateRef.current;
    if (!pending) return;
    pendingSubchatUpdateRef.current = null;
    dispatch(applyChatEvent(pending));
    callbacksRef.current.onEvent?.(pending);
  }, [dispatch]);

  const scheduleStreamDeltaFlush = useCallback(() => {
    if (streamDeltaFlushRef.current != null) return;

    const bytes = streamedBytesRef.current;
    let delayMs: number;
    if (bytes < FLUSH_TIER_FAST_BYTES) {
      delayMs = FLUSH_MS_FAST;
    } else if (bytes < FLUSH_TIER_MEDIUM_BYTES) {
      delayMs = FLUSH_MS_MEDIUM;
    } else {
      delayMs = FLUSH_MS_SLOW;
    }

    const flush = () => {
      streamDeltaFlushRef.current = null;
      flushPendingStreamDelta();
    };

    if (delayMs <= 0) {
      const frameHandle = requestNextFrame(flush);
      if (frameHandle) {
        streamDeltaFlushRef.current = frameHandle;
        return;
      }
    }

    streamDeltaFlushRef.current = {
      type: "timeout",
      id: setTimeout(flush, Math.max(delayMs, 0)),
    };
  }, [flushPendingStreamDelta]);

  const scheduleSubchatFlush = useCallback(() => {
    if (subchatFlushRef.current != null) return;

    const flush = () => {
      subchatFlushRef.current = null;
      flushPendingSubchatUpdate();
    };

    subchatFlushRef.current = {
      type: "timeout",
      id: setTimeout(flush, SUBCHAT_FLUSH_MS),
    };
  }, [flushPendingSubchatUpdate]);

  const enqueueStreamDelta = useCallback(
    (envelope: Extract<ChatEventEnvelope, { type: "stream_delta" }>) => {
      // streamedBytesRef: total chars seen this stream (never decrements),
      // drives flush-tier selection.
      // pendingBytesRef: chars currently buffered, updated precisely after
      // merge/replace decision — drives the force-flush cap.
      let deltaTextLen = 0;
      for (const op of envelope.ops) {
        if (op.op === "append_content" || op.op === "append_reasoning") {
          deltaTextLen += op.text.length;
        }
      }
      streamedBytesRef.current += deltaTextLen;

      const pending = pendingStreamDeltaRef.current;
      if (pending && pending.message_id === envelope.message_id) {
        const mergedOpsLen = pending.ops.length + envelope.ops.length;
        if (mergedOpsLen <= MAX_MERGED_DELTA_OPS) {
          // Merging: add incoming chars to existing pending buffer
          pending.seq = envelope.seq;
          pending.ops.push(...envelope.ops);
          pendingBytesRef.current += deltaTextLen;
        } else {
          // Too many ops: flush existing, start fresh with incoming envelope
          flushPendingStreamDelta(); // resets pendingBytesRef to 0
          pendingStreamDeltaRef.current = envelope;
          pendingBytesRef.current = deltaTextLen;
        }
      } else {
        // Different message or no pending: flush existing, start with incoming
        flushPendingStreamDelta(); // resets pendingBytesRef to 0
        pendingStreamDeltaRef.current = envelope;
        pendingBytesRef.current = deltaTextLen;
      }

      // Force immediate flush if *buffered* (not total) chars exceed the cap
      if (pendingBytesRef.current > MAX_BUFFERED_BYTES) {
        clearStreamDeltaFlush();
        flushPendingStreamDelta();
        return;
      }

      scheduleStreamDeltaFlush();
    },
    [flushPendingStreamDelta, scheduleStreamDeltaFlush, clearStreamDeltaFlush],
  );

  const enqueueSubchatUpdate = useCallback(
    (envelope: Extract<ChatEventEnvelope, { type: "subchat_update" }>) => {
      const pending = pendingSubchatUpdateRef.current;
      if (
        pending &&
        pending.tool_call_id === envelope.tool_call_id &&
        pending.chat_id === envelope.chat_id
      ) {
        pendingSubchatUpdateRef.current = {
          ...pending,
          seq: envelope.seq,
          subchat_id: envelope.subchat_id,
          attached_files: envelope.attached_files ?? pending.attached_files,
        };
      } else {
        flushPendingSubchatUpdate();
        pendingSubchatUpdateRef.current = envelope;
      }

      scheduleSubchatFlush();
    },
    [flushPendingSubchatUpdate, scheduleSubchatFlush],
  );

  const cleanup = useCallback(() => {
    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current);
      reconnectTimeoutRef.current = null;
    }
    clearStreamDeltaFlush();
    clearSubchatFlush();
    pendingStreamDeltaRef.current = null;
    pendingSubchatUpdateRef.current = null;
    streamedBytesRef.current = 0;
    pendingBytesRef.current = 0;
    if (unsubscribeRef.current) {
      unsubscribeRef.current();
      unsubscribeRef.current = null;
    }
    connectingRef.current = false;
  }, [clearStreamDeltaFlush, clearSubchatFlush]);

  const markSseError = useCallback(
    (err: Error) => {
      if (!chatId) return;
      dispatch(markThreadSseError({ id: chatId, error: err.message }));
    },
    [chatId, dispatch],
  );

  const scheduleReconnect = useCallback(
    (delayMs: number) => {
      if (!autoReconnect || !enabled || !chatId || !port) return;

      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current);
      }

      reconnectTimeoutRef.current = setTimeout(() => {
        connectRef.current();
      }, delayMs);
    },
    [autoReconnect, enabled, chatId, port],
  );

  const connect = useCallback(() => {
    if (!chatId || !port || !enabled) return;
    if (connectingRef.current) return;

    cleanup();
    connectingRef.current = true;
    lastSeqRef.current = 0n;
    setStatus("connecting");
    setError(null);

    unsubscribeRef.current = subscribeToChatEvents(
      chatId,
      port,
      {
        onEvent: (envelope) => {
          try {
            const seq = BigInt(envelope.seq);
            if (envelope.type === "snapshot") {
              if (DEBUG) {
                // eslint-disable-next-line no-console
                console.log(
                  "[SSE] Received snapshot event, seq:",
                  envelope.seq,
                  "messages:",
                  (envelope as { messages?: unknown[] }).messages?.length ??
                    "?",
                );
              }
              streamedBytesRef.current = 0;
              pendingBytesRef.current = 0;
              lastSeqRef.current = seq;
            } else {
              if (seq <= lastSeqRef.current) {
                return;
              }
              if (seq > lastSeqRef.current + 1n) {
                if (DEBUG) {
                  // eslint-disable-next-line no-console
                  console.log(
                    "[SSE] Sequence gap detected, reconnecting. Expected:",
                    (lastSeqRef.current + 1n).toString(),
                    "Got:",
                    envelope.seq,
                  );
                }
                flushPendingStreamDelta();
                flushPendingSubchatUpdate();
                cleanup();
                setStatus("disconnected");
                scheduleReconnect(0);
                return;
              }
              lastSeqRef.current = seq;
            }
            lastActivityAtRef.current = Date.now();
            if (envelope.type === "stream_delta") {
              enqueueStreamDelta(envelope);
            } else if (envelope.type === "subchat_update") {
              flushPendingStreamDelta();
              enqueueSubchatUpdate(envelope);
            } else {
              clearSubchatFlush();
              flushPendingSubchatUpdate();
              flushPendingStreamDelta();
              if (envelope.type === "stream_finished") {
                streamedBytesRef.current = 0;
                pendingBytesRef.current = 0;
              }
              dispatch(applyChatEvent(envelope));
              callbacksRef.current.onEvent?.(envelope);
            }
          } catch (err) {
            // Error processing event - likely malformed data
            callbacksRef.current.onError?.(
              err instanceof Error ? err : new Error(String(err)),
            );
          }
        },
        onConnected: () => {
          connectingRef.current = false;
          setStatus("connected");
          setError(null);
          callbacksRef.current.onConnected?.();
        },
        onDisconnected: () => {
          flushPendingStreamDelta();
          clearSubchatFlush();
          flushPendingSubchatUpdate();
          connectingRef.current = false;
          setStatus("disconnected");
          callbacksRef.current.onDisconnected?.();
          scheduleReconnect(reconnectDelay);
        },
        onError: (err) => {
          flushPendingStreamDelta();
          clearSubchatFlush();
          flushPendingSubchatUpdate();
          markSseError(err);
          connectingRef.current = false;
          setStatus("disconnected");
          setError(err);
          callbacksRef.current.onError?.(err);
          cleanup();
          scheduleReconnect(reconnectDelay);
        },
      },
      apiKey ?? undefined,
    );
  }, [
    chatId,
    port,
    apiKey,
    enabled,
    cleanup,
    clearSubchatFlush,
    enqueueSubchatUpdate,
    enqueueStreamDelta,
    flushPendingSubchatUpdate,
    flushPendingStreamDelta,
    markSseError,
    dispatch,
    scheduleReconnect,
    reconnectDelay,
  ]);

  connectRef.current = connect;

  const disconnect = useCallback(() => {
    cleanup();
    setStatus("disconnected");
  }, [cleanup]);

  const reconnect = useCallback(() => {
    if (DEBUG)
      console.log("[SSE] Manual reconnect triggered for chat:", chatId); // eslint-disable-line no-console
    setTimeout(() => {
      connect();
    }, 50);
  }, [connect, chatId]);

  useEffect(() => {
    if (chatId && enabled) {
      connect();
    } else {
      disconnect();
    }

    return cleanup;
  }, [chatId, enabled, connect, disconnect, cleanup]);

  useEffect(() => {
    if (status === "connected" && chatId && enabled) {
      if (DEBUG)
        console.log("[SSE] Port changed, reconnecting for chat:", chatId); // eslint-disable-line no-console
      connect();
    }
  }, [port]); // eslint-disable-line react-hooks/exhaustive-deps

  // Listen for SSE refresh requests (e.g., after trajectory transform)
  const sseRefreshRequested = useAppSelector(selectSseRefreshRequested);
  useEffect(() => {
    if (sseRefreshRequested === chatId && enabled) {
      // eslint-disable-next-line no-console
      if (DEBUG) console.log("[SSE] Refresh requested for chat:", chatId);
      reconnect();
      dispatch(clearSseRefreshRequest());
    }
  }, [sseRefreshRequested, chatId, enabled, dispatch, reconnect]);

  useEffect(() => {
    const handleVisibilityChange = () => {
      if (document.visibilityState !== "visible") return;
      if (!chatId || !enabled) return;

      const lastActivity = lastActivityAtRef.current;
      const isStale =
        lastActivity > 0 && Date.now() - lastActivity > STALE_THRESHOLD_MS;

      if (isStale && unsubscribeRef.current) {
        reconnect();
      }
    };

    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [chatId, enabled, reconnect]);

  return {
    status,
    error,
    lastSeq: lastSeqRef.current.toString(),
    connect,
    disconnect,
    reconnect,
    isConnected: status === "connected",
    isConnecting: status === "connecting",
  };
}

export default useChatSubscription;
