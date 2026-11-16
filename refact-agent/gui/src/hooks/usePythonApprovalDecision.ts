import { useCallback, useEffect, useRef, useSyncExternalStore } from "react";
import { useStore } from "react-redux";
import type { AppStore } from "../app/store";
import { removePythonApproval } from "../features/Chat/Thread/actions";
import { selectPythonApprovals } from "../features/Chat/Thread/selectors";
import { selectApiKey, selectConfig } from "../features/Config/configSlice";
import {
  ChatCommandHttpError,
  sendPythonApprovalDecision,
} from "../services/refact/chatCommands";
import type { PythonApprovalDecisionKind } from "../services/refact/types";
import { shortText } from "../components/ChatContent/ToolCard/safeText";

type DecisionState = { pending: boolean; error?: string; expired?: boolean };
type Entry = {
  chatId: string;
  requestId: string;
  state: DecisionState;
  listeners: Set<() => void>;
};
type Registry = {
  /** chatId → requestId → entry; entries exist only while a prompt is mounted or a decision is in flight. */
  chats: Map<string, Map<string, Entry>>;
  unsubscribe?: () => void;
};
const registries = new WeakMap<AppStore, Registry>();
const IDLE: DecisionState = { pending: false };
const RETRY_ERROR = "Unable to send this approval decision. Please try again.";
const REFUSAL_MAX_CHARS = 300;

/** The engine's reason for a refused (4xx) decision, else the retry line. */
function decisionError(cause: unknown): string {
  if (
    !(cause instanceof ChatCommandHttpError) ||
    cause.status < 400 ||
    cause.status >= 500
  )
    return RETRY_ERROR;
  const reason = shortText(
    engineMessage(cause.body).replace(/\s+/g, " ").trim(),
    REFUSAL_MAX_CHARS,
  ).shown;
  return reason ? `The engine refused this decision: ${reason}` : RETRY_ERROR;
}

function engineMessage(body: string): string {
  try {
    const parsed: unknown = JSON.parse(body);
    if (
      typeof parsed === "object" &&
      parsed !== null &&
      "error" in parsed &&
      typeof parsed.error === "string"
    )
      return parsed.error;
    if (typeof parsed === "string") return parsed;
    return "";
  } catch {
    return body.trimStart().startsWith("<") ? "" : body;
  }
}

function pendingIds(store: AppStore, chatId: string): ReadonlySet<string> {
  return new Set(
    selectPythonApprovals(store.getState(), chatId).map(
      (request) => request.request_id,
    ),
  );
}

function notify(entry: Entry) {
  entry.listeners.forEach((listener) => listener());
}

function drop(registry: Registry, entry: Entry) {
  const chat = registry.chats.get(entry.chatId);
  if (chat?.get(entry.requestId) !== entry) return;
  chat.delete(entry.requestId);
  if (chat.size === 0) registry.chats.delete(entry.chatId);
  if (registry.chats.size === 0) {
    registry.unsubscribe?.();
    registry.unsubscribe = undefined;
  }
}

/**
 * Watches the store only while entries exist. Per update it reads each
 * tracked chat's pending requests once (memoized selector); a request that
 * left the store resets its lock (SSE removal confirms a decision).
 */
function ensureSubscribed(store: AppStore, registry: Registry) {
  if (registry.unsubscribe) return;
  registry.unsubscribe = store.subscribe(() => {
    for (const [chatId, entries] of registry.chats) {
      const pending = pendingIds(store, chatId);
      for (const entry of entries.values()) {
        if (pending.has(entry.requestId)) continue;
        if (entry.state.pending) {
          entry.state = IDLE;
          notify(entry);
        }
        if (entry.listeners.size === 0) drop(registry, entry);
      }
    }
  });
}

function getEntry(store: AppStore, chatId: string, requestId: string) {
  let registry = registries.get(store);
  if (!registry) {
    registry = { chats: new Map() };
    registries.set(store, registry);
  }
  let chat = registry.chats.get(chatId);
  if (!chat) {
    chat = new Map();
    registry.chats.set(chatId, chat);
  }
  let entry = chat.get(requestId);
  if (!entry) {
    entry = { chatId, requestId, state: IDLE, listeners: new Set() };
    chat.set(requestId, entry);
  }
  ensureSubscribed(store, registry);
  return { registry, entry };
}

/** Number of tracked entries for a store; exported for leak tests. */
export function trackedPythonApprovalEntries(store: AppStore): number {
  let total = 0;
  for (const chat of registries.get(store)?.chats.values() ?? [])
    total += chat.size;
  return total;
}

// Scope locks to the Redux store, not a mounted prompt: a successful command
// stays locked until SSE confirms it by removing the request.
export function usePythonApprovalDecision(
  chatId: string,
  requestId: string,
  onExpired: () => void,
) {
  const store = useStore() as AppStore;
  const onExpiredRef = useRef(onExpired);
  useEffect(() => {
    onExpiredRef.current = onExpired;
  }, [onExpired]);
  const subscribe = useCallback(
    (onChange: () => void) => {
      const { registry, entry } = getEntry(store, chatId, requestId);
      const listener = () => {
        if (entry.state.expired) onExpiredRef.current();
        onChange();
      };
      entry.listeners.add(listener);
      return () => {
        entry.listeners.delete(listener);
        // Keep in-flight, failed or expired entries until their request leaves
        // the store; the store watcher drops them then.
        if (entry.listeners.size === 0 && entry.state === IDLE)
          drop(registry, entry);
      };
    },
    [store, chatId, requestId],
  );
  const state = useSyncExternalStore(
    subscribe,
    () =>
      registries.get(store)?.chats.get(chatId)?.get(requestId)?.state ?? IDLE,
  );
  const decide = async (decision: PythonApprovalDecisionKind) => {
    const { registry, entry } = getEntry(store, chatId, requestId);
    if (entry.state.pending || entry.state.expired) return;
    const update = (next: DecisionState) => {
      entry.state = next;
      notify(entry);
    };
    update({ pending: true });
    const root = store.getState();
    try {
      await sendPythonApprovalDecision(
        chatId,
        requestId,
        decision,
        selectConfig(root),
        selectApiKey(root) ?? undefined,
      );
      if (!pendingIds(store, chatId).has(requestId)) {
        update(IDLE);
        drop(registry, entry);
      }
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      if (
        cause instanceof ChatCommandHttpError &&
        cause.status === 400 &&
        /unknown.*request|request.*unknown/i.test(message)
      ) {
        update({ pending: false, expired: true });
        store.dispatch(removePythonApproval({ id: chatId, requestId }));
      } else {
        update({ pending: false, error: decisionError(cause) });
      }
      if (!pendingIds(store, chatId).has(requestId)) drop(registry, entry);
    }
  };
  return { ...state, decide };
}
