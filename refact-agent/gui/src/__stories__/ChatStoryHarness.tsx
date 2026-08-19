import React, { useState } from "react";
import { Provider } from "react-redux";
import { Flex } from "@radix-ui/themes";
import { setUpStore, type RootState } from "../app/store";
import { Theme } from "../components/Theme";
import { AbortControllerProvider } from "../contexts/AbortControllers";
import { ChatThreadProvider } from "../features/Chat/Thread";
import type { ChatThread } from "../features/Chat/Thread";
import type { ChatMessages } from "../services/refact";
import {
  makeChatSlice,
  makeChatThread,
  type ThreadRuntime,
} from "./chatStoryState";

const DEFAULT_CONFIG: RootState["config"] = {
  apiKey: "test",
  host: "web",
  lspPort: 8001,
  dev: true,
  themeProps: { appearance: "dark" },
};

export type ChatStoryHarnessProps = {
  children: React.ReactNode;
  thread?: ChatThread;
  messages?: ChatMessages;
  config?: RootState["config"];
  extraState?: Partial<RootState>;
  runtime?: Partial<ThreadRuntime>;
  height?: string;
};

export const ChatStoryHarness: React.FC<ChatStoryHarnessProps> = ({
  children,
  thread,
  messages,
  config,
  extraState,
  runtime,
  height = "100dvh",
}) => {
  const [threadData] = useState(
    () => thread ?? makeChatThread({ messages: messages ?? [] }),
  );
  const [store] = useState(() =>
    setUpStore({
      connection: {
        browserOnline: true,
        backendStatus: "online",
        backendLastOkAt: Date.now(),
        backendError: null,
        sseConnections: {},
        visibleChatMounts: {},
        suspended: false,
      },
      ...extraState,
      config: config ?? DEFAULT_CONFIG,
      chat: makeChatSlice(threadData, runtime),
    }),
  );

  return (
    <Provider store={store}>
      <Theme>
        <AbortControllerProvider>
          <ChatThreadProvider chatId={threadData.id}>
            <Flex direction="column" align="stretch" height={height}>
              {children}
            </Flex>
          </ChatThreadProvider>
        </AbortControllerProvider>
      </Theme>
    </Provider>
  );
};
