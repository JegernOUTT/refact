import { collectTabIds } from "../ChatPanes/panesTree";
import { useEffect } from "react";
import { selectCurrentThreadId } from "../Chat/Thread";
import { useAppDispatch, useAppSelector, useConfig } from "../../hooks";
import {
  buildApiUrl,
  hasUsableEngineEndpoint,
} from "../../services/refact/apiUrl";
import { appsApi } from "../../services/refact/apps";
import { publishAppEvent, readAppStream } from "./appStream";

export function useAppsStream() {
  const dispatch = useAppDispatch();
  const config = useConfig();
  const activeChat = useAppSelector(selectCurrentThreadId);
  const views = useAppSelector((state) => state.apps.views);
  const workspace = useAppSelector((state) => state.workspace);
  const open = new Set([
    ...workspace.tabs,
    ...Object.values(workspace.groups).flatMap((group) =>
      group ? collectTabIds(group.root) : [],
    ),
  ]);
  const scopes = JSON.stringify(
    Array.from(
      new Set([
        activeChat || null,
        ...Object.entries(views)
          .filter(([key]) => open.has(key))
          .map(([, view]) => view?.scopeChatId ?? view?.chatId ?? null),
      ]),
    ).sort(),
  );
  const token = config.apiKey;
  const ready = hasUsableEngineEndpoint(config);
  const baseUrl = buildApiUrl(config, "/v1/apps/subscribe");
  useEffect(() => {
    if (!ready) return;
    const abort = new AbortController();
    const timers = new Set<ReturnType<typeof setTimeout>>();
    const chatIds = JSON.parse(scopes) as (string | null)[];
    const connect = async (chatId: string | null) => {
      try {
        const url = chatId
          ? `${baseUrl}?chat_id=${encodeURIComponent(chatId)}`
          : baseUrl;
        const response = await fetch(url, {
          signal: abort.signal,
          headers: token ? { Authorization: `Bearer ${token}` } : {},
        });
        await readAppStream(response, abort.signal, (event) => {
          if (event.kind === "snapshot" || event.kind === "reset")
            dispatch(appsApi.util.invalidateTags(["Apps", "AppDocument"]));
          publishAppEvent({ ...event, hostChatId: chatId });
        });
      } catch {
        /* Reconnect with a fresh bounded snapshot. */
      }
      if (!abort.signal.aborted) {
        const timer = setTimeout(() => {
          timers.delete(timer);
          void connect(chatId);
        }, 3000);
        timers.add(timer);
      }
    };
    chatIds.forEach((chatId) => void connect(chatId));
    return () => {
      abort.abort();
      timers.forEach(clearTimeout);
    };
  }, [dispatch, scopes, baseUrl, token, ready]);
}
