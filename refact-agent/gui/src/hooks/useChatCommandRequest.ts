import { useCallback, useContext, useEffect, useRef, useState } from "react";
import { useStore } from "react-redux";
import { v4 as uuidv4 } from "uuid";
import type { AppStore, RootState } from "../app/store";
import { useAppSelector } from "./useAppSelector";
import { selectApiKey, selectConfig } from "../features/Config/configSlice";
import {
  selectChatErrorById,
  selectCommandCancelledByThreadAndId,
  selectQueuedItemsById,
  selectCommandAckByThreadAndId,
  selectCurrentThreadId,
} from "../features/Chat/Thread/selectors";
import { ChatThreadContext } from "../features/Chat/Thread/ChatThreadContext";
import {
  markQueuedCommandCancelled,
  requestSseRefresh,
} from "../features/Chat/Thread/actions";
import { getEngineEndpointIdentity } from "../services/refact/apiUrl";
import {
  ChatCommandHttpError,
  cancelQueuedItem,
  sendChatCommand,
  type ChatCommandBase,
} from "../services/refact/chatCommands";

const ACK_WAIT_MS = 15_000;
const STATUS_REFRESH_MS = 2_000;
const useAppStore = useStore.withTypes<AppStore>();

export type ChatCommandRequestPhase =
  | "idle"
  | "sending"
  | "awaiting_ack"
  | "queued"
  | "awaiting_applied"
  | "applied"
  | "rejected"
  | "uncertain"
  | "cancelled";

type Request = {
  id: string;
  scope: string;
  sent: boolean;
  observing: boolean;
  requiresAppliedAck: boolean;
  initialWitness: boolean;
  witnessed: boolean;
  controller: AbortController;
  applied: (state: RootState) => boolean;
  sourceValid: (state: RootState) => boolean;
  initialError: string | null;
  check: () => void;
  cancel: (message?: string, unknown?: boolean) => void;
};

function ackError(result: unknown): string {
  if (result && typeof result === "object" && "error" in result) {
    if (typeof result.error === "string") return result.error;
  }
  return "The server rejected this request. Refresh the chat and try again.";
}

export function useChatCommandRequest(
  chatId: string,
  source: string,
  matchesSource: (state: RootState) => boolean,
) {
  const store = useAppStore();
  const explicitThreadId = useContext(ChatThreadContext);
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);
  const endpoint = getEngineEndpointIdentity(config);
  const scope = JSON.stringify([endpoint, chatId, source]);
  const currentScope = useRef(scope);
  currentScope.current = scope;
  const mounted = useRef(true);
  const active = useRef<Request | null>(null);
  const [phase, setPhase] = useState<ChatCommandRequestPhase>("idle");
  const [queuedCommandId, setQueuedCommandId] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  const isMountedScope = useCallback(
    () =>
      mounted.current &&
      currentScope.current === scope &&
      (explicitThreadId !== null ||
        selectCurrentThreadId(store.getState()) === chatId) &&
      getEngineEndpointIdentity(selectConfig(store.getState())) === endpoint &&
      selectApiKey(store.getState()) === apiKey,
    [apiKey, chatId, endpoint, explicitThreadId, scope, store],
  );

  const isCurrent = useCallback(
    () => isMountedScope() && matchesSource(store.getState()),
    [isMountedScope, matchesSource, store],
  );

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      active.current?.cancel();
      active.current = null;
    };
  }, [scope]);

  const observe = useCallback(
    (request: Request): Promise<boolean> =>
      new Promise((resolve) => {
        request.observing = true;
        let done = false;
        let unsubscribe: () => void = () => undefined;
        const finish = (
          applied: boolean,
          message?: string,
          unknown = false,
          terminalPhase?: ChatCommandRequestPhase,
        ) => {
          if (done) return;
          done = true;
          request.observing = false;
          unsubscribe();
          clearTimeout(timeout);
          clearInterval(refresh);
          request.controller.abort();
          if (unknown) request.sent = true;
          if (!unknown && active.current === request) active.current = null;
          if (isMountedScope()) {
            setPhase(
              terminalPhase ??
                (applied
                  ? "applied"
                  : unknown
                    ? "uncertain"
                    : message
                      ? "rejected"
                      : "idle"),
            );
            setQueuedCommandId(null);
            setPending(false);
            setUncertain(unknown);
            setError(message ?? null);
            setStatus(null);
          }
          resolve(applied);
        };
        const check = () => {
          if (done) return;
          if (!isMountedScope()) {
            finish(false);
            return;
          }
          const state = store.getState();
          const ack = selectCommandAckByThreadAndId(state, chatId, request.id);
          if (ack && !ack.accepted) {
            finish(false, ackError(ack.result));
            return;
          }
          if (selectCommandCancelledByThreadAndId(state, chatId, request.id)) {
            finish(false, undefined, false, "cancelled");
            return;
          }
          if (request.applied(state) && !request.initialWitness)
            request.witnessed = true;
          const queued = selectQueuedItemsById(state, chatId).some(
            (item) => item.client_request_id === request.id,
          );
          setQueuedCommandId(queued ? request.id : null);
          if (queued) {
            setPhase("queued");
            setStatus("Request queued; waiting for the server to process it.");
            return;
          }
          const appliedAck =
            !!ack?.result &&
            typeof ack.result === "object" &&
            "applied" in ack.result &&
            ack.result.applied === true;
          if (
            request.sent &&
            ack?.accepted &&
            (request.requiresAppliedAck
              ? appliedAck && request.applied(state)
              : request.witnessed)
          ) {
            finish(true);
          } else {
            const runtimeError = selectChatErrorById(state, chatId);
            if (
              !request.witnessed &&
              runtimeError &&
              runtimeError !== request.initialError
            ) {
              finish(
                false,
                `${runtimeError} Check status before retrying.`,
                true,
              );
              return;
            }
            if (!request.sourceValid(state) && !request.witnessed) {
              finish(
                false,
                "The source changed. Check status before responding again.",
                true,
              );
              return;
            }
            if (!request.sent) return;
            setPhase(ack?.accepted ? "awaiting_applied" : "awaiting_ack");
            setStatus(
              ack?.accepted
                ? "Request accepted; awaiting the server's applied state."
                : "Request sent; awaiting server acknowledgment.",
            );
          }
        };
        request.check = check;
        request.cancel = (message, unknown) => finish(false, message, unknown);
        unsubscribe = store.subscribe(check);
        const checkTimeout = () => {
          if (
            isMountedScope() &&
            selectQueuedItemsById(store.getState(), chatId).some(
              (item) => item.client_request_id === request.id,
            )
          ) {
            timeout = setTimeout(checkTimeout, ACK_WAIT_MS);
            return;
          }
          finish(
            false,
            "Outcome not confirmed. Check status before sending another request; it may already have applied.",
            true,
          );
        };
        let timeout = setTimeout(checkTimeout, ACK_WAIT_MS);
        const refresh = setInterval(() => {
          if (isMountedScope() && request.sent) {
            store.dispatch(requestSseRefresh({ chatId }));
          }
        }, STATUS_REFRESH_MS);
        check();
      }),
    [chatId, isMountedScope, store],
  );

  const run = useCallback(
    async (
      command: ChatCommandBase,
      applied: (state: RootState) => boolean,
      clientRequestId?: string,
    ): Promise<boolean> => {
      if (!isCurrent() || active.current) return false;
      const request: Request = {
        id: clientRequestId ?? uuidv4(),
        scope,
        sent: false,
        observing: false,
        requiresAppliedAck: command.type === "set_params",
        initialWitness: applied(store.getState()),
        witnessed: false,
        controller: new AbortController(),
        applied,
        sourceValid: matchesSource,
        initialError: selectChatErrorById(store.getState(), chatId),
        check: () => undefined,
        cancel: () => undefined,
      };
      active.current = request;
      setPhase("sending");
      setPending(true);
      setUncertain(false);
      setError(null);
      setStatus("Sending request…");
      const observed = observe(request);
      const send = async () => {
        try {
          await sendChatCommand(
            chatId,
            config,
            apiKey ?? undefined,
            command,
            undefined,
            request.id,
            request.controller.signal,
          );
          if (request.controller.signal.aborted) return;
          request.sent = true;
          request.check();
          if (isCurrent() && active.current === request) {
            store.dispatch(requestSseRefresh({ chatId }));
          }
        } catch (cause) {
          if (request.controller.signal.aborted) return;
          const rejected =
            cause instanceof ChatCommandHttpError &&
            cause.status >= 400 &&
            cause.status < 500 &&
            cause.status !== 408;
          if (isCurrent()) {
            const detail =
              cause instanceof Error ? cause.message : "Send failed.";
            request.sent = true;
            request.cancel(
              rejected
                ? detail
                : `${detail} Outcome unknown; check status before retrying.`,
              !rejected,
            );
          }
        }
      };
      if (isCurrent() && active.current === request) void send();
      return observed;
    },
    [apiKey, chatId, config, isCurrent, matchesSource, observe, scope, store],
  );

  const checkStatus = useCallback(async () => {
    const request = active.current;
    if (
      !isMountedScope() ||
      !request ||
      request.observing ||
      request.scope !== scope
    )
      return;
    request.initialError = selectChatErrorById(store.getState(), chatId);
    setPending(true);
    setUncertain(false);
    setError(null);
    const observed = observe(request);
    store.dispatch(requestSseRefresh({ chatId }));
    await observed;
  }, [chatId, isMountedScope, observe, scope, store]);

  const cancelQueuedRequest = useCallback(async () => {
    const request = active.current;
    if (
      !isMountedScope() ||
      !request ||
      !selectQueuedItemsById(store.getState(), chatId).some(
        (item) => item.client_request_id === request.id,
      )
    )
      return;
    try {
      const accepted = await cancelQueuedItem(
        chatId,
        request.id,
        config,
        apiKey ?? undefined,
      );
      if (!isMountedScope() || active.current !== request) return;
      if (accepted) {
        store.dispatch(
          markQueuedCommandCancelled({ chatId, clientRequestId: request.id }),
        );
      }
      if (!accepted)
        setError(
          "Cancellation was not confirmed. Check status before retrying.",
        );
      store.dispatch(requestSseRefresh({ chatId }));
      request.check();
    } catch {
      if (isMountedScope() && active.current === request)
        setError("Cancellation outcome unknown. Check status before retrying.");
    }
  }, [apiKey, chatId, config, isMountedScope, store]);

  return {
    phase,
    queuedCommandId,
    cancelQueuedRequest,
    run,
    isCurrent,
    hasUnresolvedRequest: () => active.current !== null,
    pending,
    blocked: pending || uncertain,
    uncertain,
    error,
    status,
    checkStatus,
  };
}
