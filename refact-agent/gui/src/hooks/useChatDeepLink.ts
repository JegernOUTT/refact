import { useEffect, useRef, useState } from "react";
import { useStore } from "react-redux";

import type { AppStore } from "../app/store";
import {
  closeThread,
  createChatWithId,
  restoreChat,
  switchToThread,
} from "../features/Chat/Thread";
import { chatRemovedRemotely } from "../features/History/historySlice";
import { popBackTo, push } from "../features/Pages/pagesSlice";
import { selectWorkspaceSection } from "../features/Sidebar/sidebarSlice";
import { getEngineEndpointIdentity } from "../services/refact/apiUrl";
import { trajectoriesApi } from "../services/refact/trajectories";
import { useAppSelector } from "./useAppSelector";
import { useConfig } from "./useConfig";

const useAppStore = useStore.withTypes<AppStore>();

const WORKSPACE_WAIT_MS = 8_000;
const OPEN_SETTLE_MS = 20_000;

type Phase = "waiting" | "opened" | "done" | "failed";
type Existence = "unknown" | "checking" | "exists" | "missing";
type Snapshot = "none" | "empty" | "messages";

export function readChatDeepLinkChatId(): string | null {
  const trimmed = new URLSearchParams(window.location.search)
    .get("chat")
    ?.trim();
  return trimmed ? trimmed : null;
}

export function clearChatDeepLinkParam(): void {
  const params = new URLSearchParams(window.location.search);
  if (!params.has("chat")) return;
  params.delete("chat");
  const query = params.toString();
  const nextUrl = `${window.location.pathname}${query ? `?${query}` : ""}${
    window.location.hash
  }`;
  window.history.replaceState(window.history.state, "", nextUrl);
}

function isNotFound(cause: unknown): boolean {
  return (
    typeof cause === "object" &&
    cause !== null &&
    "status" in cause &&
    cause.status === 404
  );
}

export function useChatDeepLink(ready: boolean) {
  const store = useAppStore();
  const config = useConfig();
  const enabled = config.host === "web" && config.engineServed === true;
  const [linkedChatId] = useState(readChatDeepLinkChatId);
  const targetId = enabled ? linkedChatId : null;
  const [phase, setPhase] = useState<Phase>("waiting");
  const [existence, setExistence] = useState<Existence>("unknown");
  const [error, setError] = useState<string | null>(null);
  const [workspaceWaitExpired, setWorkspaceWaitExpired] = useState(false);
  const [listed, setListed] = useState(false);
  const mounted = useRef(true);

  const workspaceLoaded =
    useAppSelector(selectWorkspaceSection).status !== "loading";
  const snapshot = useAppSelector((state): Snapshot => {
    const runtime = targetId ? state.chat.threads[targetId] : undefined;
    if (!runtime?.snapshot_received) return "none";
    return runtime.thread.messages.length > 0 ? "messages" : "empty";
  });

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (!targetId || !ready || workspaceLoaded || workspaceWaitExpired) return;
    const timeoutId = setTimeout(
      () => setWorkspaceWaitExpired(true),
      WORKSPACE_WAIT_MS,
    );
    return () => clearTimeout(timeoutId);
  }, [ready, targetId, workspaceLoaded, workspaceWaitExpired]);

  const canOpen = ready && (workspaceLoaded || workspaceWaitExpired);

  useEffect(() => {
    if (!targetId || phase !== "waiting" || !canOpen) return;
    const state = store.getState();
    const runtime = state.chat.threads[targetId];
    const historyItem = state.history.chats[targetId] as
      | (typeof state.history.chats)[string]
      | undefined;
    if (!runtime && historyItem) {
      store.dispatch(restoreChat(historyItem));
    } else {
      if (!runtime) {
        store.dispatch(
          createChatWithId({ id: targetId, openTab: false, activate: false }),
        );
      }
      store.dispatch(switchToThread({ id: targetId }));
    }
    store.dispatch(popBackTo({ name: "history" }));
    store.dispatch(push({ name: "chat" }));
    setListed(Boolean(historyItem));
    setPhase("opened");
  }, [canOpen, phase, store, targetId]);

  useEffect(() => {
    if (!targetId || phase !== "opened" || listed) return;
    if (snapshot !== "empty" || existence !== "unknown") return;
    setExistence("checking");
    const endpoint = getEngineEndpointIdentity(store.getState().config);
    void store
      .dispatch(
        trajectoriesApi.endpoints.getTrajectoryPath.initiate(targetId, {
          forceRefetch: true,
          subscribe: false,
        }),
      )
      .unwrap()
      .then(
        (): Existence => "exists",
        (cause: unknown): Existence =>
          isNotFound(cause) ? "missing" : "exists",
      )
      .then((result) => {
        if (!mounted.current) return;
        const sameEndpoint =
          getEngineEndpointIdentity(store.getState().config) === endpoint;
        setExistence(sameEndpoint ? result : "exists");
      });
  }, [existence, listed, phase, snapshot, store, targetId]);

  useEffect(() => {
    if (!targetId || phase !== "opened") return;
    const timeoutId = setTimeout(() => {
      clearChatDeepLinkParam();
      setPhase("done");
    }, OPEN_SETTLE_MS);
    return () => clearTimeout(timeoutId);
  }, [phase, targetId]);

  useEffect(() => {
    if (!targetId || phase !== "opened") return;
    if (listed || snapshot === "messages" || existence === "exists") {
      clearChatDeepLinkParam();
      setPhase("done");
      return;
    }
    if (existence !== "missing") return;
    store.dispatch(closeThread({ id: targetId, force: true }));
    store.dispatch(chatRemovedRemotely(targetId));
    store.dispatch(popBackTo({ name: "history" }));
    clearChatDeepLinkParam();
    setError(`Chat not found: ${targetId}`);
    setPhase("failed");
  }, [existence, listed, phase, snapshot, store, targetId]);

  return {
    targetId,
    resolving: targetId !== null && ready && phase === "waiting",
    pending: targetId !== null && (phase === "waiting" || phase === "opened"),
    error,
    dismissError: () => setError(null),
  };
}
