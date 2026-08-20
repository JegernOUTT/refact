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
  resolveStoryAppearance,
  type StoryAppearance,
  type ThreadRuntime,
} from "./chatStoryState";

function makeDefaultConfig(appearance: StoryAppearance): RootState["config"] {
  return {
    apiKey: "test",
    host: "web",
    lspPort: 8001,
    dev: true,
    themeProps: { appearance },
  };
}

// A story-provided config still gets the resolved appearance unless it pins
// one itself, so the toolbar global keeps working for host-specific stories.
function withStoryAppearance(
  config: RootState["config"],
  appearance: StoryAppearance,
): RootState["config"] {
  if (config.themeProps.appearance) return config;
  return { ...config, themeProps: { ...config.themeProps, appearance } };
}

export type ChatStoryHarnessProps = {
  children: React.ReactNode;
  thread?: ChatThread;
  messages?: ChatMessages;
  config?: RootState["config"];
  extraState?: Partial<RootState>;
  runtime?: Partial<ThreadRuntime>;
  height?: string;
  appearance?: StoryAppearance;
};

export const ChatStoryHarness: React.FC<ChatStoryHarnessProps> = ({
  children,
  thread,
  messages,
  config,
  extraState,
  runtime,
  height = "100dvh",
  appearance,
}) => {
  const [resolvedAppearance] = useState(() =>
    resolveStoryAppearance(appearance),
  );
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
      config: config
        ? withStoryAppearance(config, resolvedAppearance)
        : makeDefaultConfig(resolvedAppearance),
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
