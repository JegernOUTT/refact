import { useEffect, useRef } from "react";

import { useConfig } from "./useConfig";
import { useInternalNavigation } from "./useInternalNavigation";

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
  const navigation = useInternalNavigation();
  const { navigate } = navigation;
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
    void navigate(`refact://chat/${chatId}`);
  }, [navigate, isEngineServedWeb, ready]);
  return navigation;
}
