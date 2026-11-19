import { addListener } from "@reduxjs/toolkit";
import { useEffect } from "react";
import type { AppDispatch, RootState } from "../../app/store";
import { useAppDispatch } from "../../hooks";
import type { ChatEventEnvelope } from "../../services/refact/chatSubscription";
import {
  browserHintsHandled,
  fetchBrowserStatusAfterNow,
  openBrowserTabForChat,
  selectBrowserHintHandled,
} from "../Browser/browserViews";
import { applyChatEvent } from "../Chat/Thread/actions";
import { selectSnapshotReceivedById } from "../Chat/Thread/selectors";
import { isValidBrowserTabId } from "./surfaceKey";
import { selectChatHasBrowserTab } from "./workspaceSlice";

export type ProjectBrowserOpenHint = {
  toolCallId: string;
  tabId: string | null;
  tabName: string | null;
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

type ChatEventMessage = Extract<
  ChatEventEnvelope,
  { type: "message_added" }
>["message"];

function messageOpenHint(
  message: ChatEventMessage,
): ProjectBrowserOpenHint | undefined {
  if (message.role !== "tool") return undefined;
  const wire: unknown = message;
  const hint = isRecord(wire) ? wire.browser_project : undefined;
  if (!isRecord(hint) || hint.open_browser_tab !== true) return undefined;
  const { tab_id: tabId, tab_name: tabName } = hint;
  if (
    tabId !== null &&
    (typeof tabId !== "string" || !isValidBrowserTabId(tabId))
  )
    return undefined;
  const toolCallId = message.tool_call_id;
  if (!toolCallId) return undefined;
  return {
    toolCallId,
    tabId,
    tabName:
      tabId !== null && typeof tabName === "string" && tabName.trim()
        ? tabName
        : null,
  };
}

export function projectBrowserOpenHint(
  event: ChatEventEnvelope,
): ProjectBrowserOpenHint | undefined {
  if (event.type !== "message_added" && event.type !== "message_updated")
    return undefined;
  return messageOpenHint(event.message);
}

export function snapshotBrowserOpenHints(
  event: ChatEventEnvelope,
): ProjectBrowserOpenHint[] {
  if (event.type !== "snapshot" && event.type !== "snapshot_chunk") return [];
  return event.messages.flatMap((message) => messageOpenHint(message) ?? []);
}

const hintNames = (
  hints: ProjectBrowserOpenHint[],
): Record<string, string> | undefined => {
  const named = hints.flatMap(({ tabId, tabName }) =>
    tabId && tabName ? [[tabId, tabName] as const] : [],
  );
  return named.length > 0 ? Object.fromEntries(named) : undefined;
};

export function useProjectBrowserAutoOpen() {
  const dispatch = useAppDispatch();

  useEffect(() => {
    const unsubscribe = dispatch(
      addListener({
        actionCreator: applyChatEvent,
        effect: async ({ payload: event }, api) => {
          const replayed = snapshotBrowserOpenHints(event);
          if (replayed.length > 0) {
            api.dispatch(
              browserHintsHandled({
                chatId: event.chat_id,
                toolCallIds: replayed.map((item) => item.toolCallId),
                names: hintNames(replayed),
              }),
            );
            return;
          }
          const hint = projectBrowserOpenHint(event);
          if (!hint) return;
          const names = hintNames([hint]);
          const state = api.getState() as RootState;
          if (
            !selectSnapshotReceivedById(state, event.chat_id) ||
            selectBrowserHintHandled(state, event.chat_id, hint.toolCallId)
          ) {
            if (names)
              api.dispatch(
                browserHintsHandled({
                  chatId: event.chat_id,
                  toolCallIds: [],
                  names,
                }),
              );
            return;
          }
          const tabId = hint.tabId;
          api.dispatch(
            browserHintsHandled({
              chatId: event.chat_id,
              toolCallIds: [hint.toolCallId],
              names,
              hintedTabId: tabId ?? undefined,
            }),
          );
          if (!tabId) return;
          if (
            !selectChatHasBrowserTab(
              api.getState() as RootState,
              event.chat_id,
              tabId,
            )
          )
            api.dispatch(
              openBrowserTabForChat({
                chatId: event.chat_id,
                tabId,
                automatic: true,
              }),
            );
          await fetchBrowserStatusAfterNow(
            api.dispatch as AppDispatch,
            () => api.getState() as RootState,
            (done, timeoutMs) =>
              api.condition(
                (_action, current) => done(current as RootState),
                timeoutMs,
              ),
          );
        },
      }),
    );
    return () => unsubscribe({ cancelActive: true });
  }, [dispatch]);
}
