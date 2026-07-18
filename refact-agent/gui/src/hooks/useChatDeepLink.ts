import { useEffect, useRef } from "react";

import { restoreChat } from "../features/Chat/Thread";
import type { ChatHistoryItem } from "../features/History/historySlice";
import { push } from "../features/Pages/pagesSlice";
import {
  trajectoriesApi,
  trajectoryDataToChatThread,
} from "../services/refact";
import { useAppDispatch } from "./useAppDispatch";
import { useConfig } from "./useConfig";

export function consumeChatDeepLinkChatId(): string | null {
  const params = new URLSearchParams(window.location.search);
  const chatId = params.get("chat");
  if (chatId === null) return null;
  params.delete("chat");
  const query = params.toString();
  const nextUrl = `${window.location.pathname}${query ? `?${query}` : ""}${
    window.location.hash
  }`;
  window.history.replaceState(window.history.state, "", nextUrl);
  const trimmed = chatId.trim();
  return trimmed === "" ? null : trimmed;
}

export function useChatDeepLink(ready: boolean) {
  const dispatch = useAppDispatch();
  const config = useConfig();
  const handledRef = useRef(false);
  const isEngineServedWeb =
    config.host === "web" && config.engineServed === true;

  useEffect(() => {
    if (handledRef.current) return;
    if (!isEngineServedWeb || !ready) return;
    handledRef.current = true;
    const chatId = consumeChatDeepLinkChatId();
    if (chatId === null) return;
    const request = dispatch(
      trajectoriesApi.endpoints.getTrajectory.initiate(chatId, {
        forceRefetch: true,
        subscribe: false,
      }),
    );
    void request
      .unwrap()
      .then((result) => {
        const thread = trajectoryDataToChatThread(result);
        const historyItem: ChatHistoryItem = {
          ...thread,
          createdAt: result.created_at,
          updatedAt: result.updated_at,
          title: result.title,
          isTitleGenerated: result.isTitleGenerated,
        };
        dispatch(restoreChat(historyItem));
        dispatch(push({ name: "chat" }));
      })
      .catch(() => undefined);
  }, [dispatch, isEngineServedWeb, ready]);
}
