import { useState } from "react";
import { Meta, StoryObj } from "@storybook/react";

import { ChatLinks } from "./ChatLinks";
import { setUpStore } from "../../app/store";
import { Provider } from "react-redux";
import { Theme } from "../Theme";
import { type HttpHandler } from "msw";
import {
  chatLinks,
  goodCaps,
  goodPing,
  goodChatModes,
} from "../../__fixtures__/msw";
import { CHAT_CONFIG_THREAD } from "../../__fixtures__";
import { ChatThreadProvider } from "../../features/Chat/Thread";

const Template = () => {
  const chatId = CHAT_CONFIG_THREAD.current_thread_id;
  // ChatLinks only requests `/v1/links` when follow-ups are enabled, the
  // backend is reachable and the thread is idle, so all of that has to be
  // preloaded here or the component renders `null`.
  const [store] = useState(() =>
    setUpStore({
      config: {
        apiKey: "test",
        host: "web",
        lspPort: 8001,
        dev: true,
        themeProps: {},
      },
      connection: {
        browserOnline: true,
        backendStatus: "online",
        backendLastOkAt: Date.now(),
        backendError: null,
        sseConnections: {},
        visibleChatMounts: {},
        suspended: false,
      },
      chat: {
        ...CHAT_CONFIG_THREAD,
        follow_ups_enabled: true,
      },
    }),
  );

  return (
    <Provider store={store}>
      <Theme>
        <ChatThreadProvider chatId={chatId}>
          <div
            style={{
              padding: 16,
              display: "flex",
              flexWrap: "wrap",
              gap: 8,
            }}
          >
            <ChatLinks />
          </div>
        </ChatThreadProvider>
      </Theme>
    </Provider>
  );
};

const meta = {
  title: "Components/ChatLinks",
  component: Template,
  argTypes: {
    //...
  },
  parameters: {
    msw: {
      handlers: [goodPing, goodCaps, goodChatModes, chatLinks],
    },
  },
} satisfies Meta<
  typeof Template & { parameters: { msw: { handlers: HttpHandler[] } } }
>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Default: Story = {};
