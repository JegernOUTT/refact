/* eslint-disable react-refresh/only-export-components */
import React, { createContext, useContext } from "react";
import { useAppSelector } from "../../../hooks";
import { selectCurrentThreadId } from "./selectors";

export const ChatThreadContext = createContext<string | null>(null);

export function ChatThreadProvider({
  chatId,
  children,
}: {
  chatId: string;
  children: React.ReactNode;
}) {
  return (
    <ChatThreadContext.Provider value={chatId}>
      {children}
    </ChatThreadContext.Provider>
  );
}

export function useThreadId(): string {
  const contextId = useContext(ChatThreadContext);
  const currentThreadId = useAppSelector(selectCurrentThreadId);
  return contextId ?? currentThreadId;
}
