import { useMemo } from "react";
import { useAppSelector } from "./useAppSelector";
import type { ChatHistoryItem } from "../features/History/historySlice";

export type SessionState = NonNullable<ChatHistoryItem["session_state"]>;

export function useChatSessionStates(): Record<string, SessionState> {
  const historyChats = useAppSelector((state) => state.history.chats);
  const chatThreads = useAppSelector((state) => state.chat.threads);

  return useMemo(() => {
    const states: Record<string, SessionState> = {};

    for (const [id, runtime] of Object.entries(chatThreads)) {
      if (!runtime) continue;
      if (runtime.streaming) {
        states[id] = "generating";
      } else if (runtime.confirmation.pause) {
        states[id] = "paused";
      } else if (runtime.waiting_for_response) {
        states[id] = "executing_tools";
      } else if (runtime.error) {
        states[id] = "error";
      }
    }

    for (const chat of Object.values(historyChats)) {
      if (chat.id in states) continue;
      if (chat.session_state && chat.session_state !== "idle") {
        states[chat.id] = chat.session_state;
      }
    }

    return states;
  }, [historyChats, chatThreads]);
}
