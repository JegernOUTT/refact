import React from "react";
import type { Config } from "../Config/configSlice";
import { Chat as ChatComponent } from "../../components/Chat";
import { useAppSelector } from "../../hooks";
import {
  ChatThreadProvider,
  selectCurrentThreadId,
  selectHasUncalledToolsById,
} from "./Thread";

export type ChatProps = {
  host: Config["host"];
  tabbed: Config["tabbed"];
  style?: React.CSSProperties;
  backFromChat: () => void;
  chatId?: string;
};

export const Chat: React.FC<ChatProps> = ({
  style,
  backFromChat,
  host,
  tabbed,
  chatId,
}) => {
  const currentThreadId = useAppSelector(selectCurrentThreadId);
  const resolvedChatId = chatId ?? currentThreadId;

  const sendToSideBar = () => {
    // TODO:
  };

  const maybeSendToSideBar =
    host === "vscode" && tabbed ? sendToSideBar : undefined;

  const unCalledTools = useAppSelector((state) =>
    selectHasUncalledToolsById(state, resolvedChatId),
  );

  return (
    <ChatThreadProvider chatId={resolvedChatId}>
      <ChatComponent
        style={style}
        host={host}
        tabbed={tabbed}
        backFromChat={backFromChat}
        unCalledTools={unCalledTools}
        maybeSendToSidebar={maybeSendToSideBar}
      />
    </ChatThreadProvider>
  );
};
