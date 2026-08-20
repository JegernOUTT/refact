/* eslint-disable @typescript-eslint/no-non-null-assertion */
import type { Meta, StoryObj } from "@storybook/react";
import { Chat } from "./Chat";
import { ChatThread } from "../../features/Chat/Thread/types";
import { RootState } from "../../app/store";
import {
  CHAT_CONFIG_THREAD,
  CHAT_WITH_KNOWLEDGE_TOOL,
} from "../../__fixtures__";

import { chatLinks, goodTools } from "../../__fixtures__/msw";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import { CHAT_STORY_MSW_HANDLERS } from "../../__stories__/chatStoryState";
import { makeChatThread } from "../../__stories__/chatStoryState";

type ChatStoryProps = {
  thread?: ChatThread;
  config?: RootState["config"];
};

// The shared harness owns the store, Theme, abort controllers and the
// full-height flex column, so the chat shell fills the preview viewport
// instead of being pushed past it by story padding.
const Template = ({ thread, config }: ChatStoryProps) => (
  <ChatStoryHarness thread={thread ?? makeChatThread()} config={config}>
    <Chat
      unCalledTools={false}
      host="web"
      tabbed={false}
      backFromChat={() => ({})}
      maybeSendToSidebar={() => ({})}
    />
  </ChatStoryHarness>
);

// MSW resolves the first matching handler, so the story-specific ones are
// prepended to the shared chat polling set.
const CHAT_HANDLERS = [chatLinks, ...CHAT_STORY_MSW_HANDLERS];
const CHAT_HANDLERS_WITH_TOOLS = [goodTools, ...CHAT_HANDLERS];

const meta: Meta<typeof Template> = {
  title: "Chat",
  component: Template,
  parameters: {
    layout: "fullscreen",
    msw: {
      handlers: CHAT_HANDLERS_WITH_TOOLS,
    },
  },
  argTypes: {},
};

export default meta;

type Story = StoryObj<typeof Template>;

export const Primary: Story = {};

export const Configuration: Story = {
  args: {
    thread:
      CHAT_CONFIG_THREAD.threads[CHAT_CONFIG_THREAD.current_thread_id]!.thread,
  },
};

export const IDE: Story = {
  args: {
    config: {
      host: "ide",
      lspPort: 8001,
      themeProps: {},
      features: { vecdb: true },
    },
  },

  parameters: {
    msw: {
      handlers: CHAT_HANDLERS,
    },
  },
};

export const Knowledge: Story = {
  args: {
    thread: CHAT_WITH_KNOWLEDGE_TOOL,
    config: {
      host: "ide",
      lspPort: 8001,
      themeProps: {},
      features: {
        vecdb: true,
      },
    },
  },
  parameters: {
    msw: {
      handlers: CHAT_HANDLERS,
    },
  },
};

export const EmptySpaceAtBottom: Story = {
  args: {
    thread: makeChatThread({
      messages: [
        {
          role: "user",
          content: "Hello",
        },
        {
          role: "assistant",
          content: "Hi",
        },
        {
          role: "user",
          content: "👋",
        },
      ],
    }),
  },

  parameters: {
    msw: {
      handlers: CHAT_HANDLERS,
    },
  },
};

export const UserMessageEmptySpaceAtBottom: Story = {
  args: {
    thread: makeChatThread({
      messages: [
        {
          role: "user",
          content: "Hello",
        },
        {
          role: "assistant",
          content: "Hi",
        },
        {
          role: "user",
          content: "👋",
        },
        { role: "assistant", content: "👋" },
        {
          role: "user",
          content: "Hello",
        },
        {
          role: "assistant",
          content: "Hi",
        },
        {
          role: "user",
          content: "👋",
        },
        { role: "assistant", content: "👋" },
        {
          role: "user",
          content: "Hello",
        },
        {
          role: "assistant",
          content: "Hi",
        },
        {
          role: "user",
          content: "👋",
        },
        { role: "assistant", content: "👋" },
        {
          role: "user",
          content: "Hello",
        },
        {
          role: "assistant",
          content: "Hi",
        },
        {
          role: "user",
          content: "👋",
        },
        { role: "assistant", content: "👋" },
      ],
    }),
  },

  parameters: {
    msw: {
      handlers: CHAT_HANDLERS,
    },
  },
};

export const CompressButton: Story = {
  args: {
    thread: makeChatThread({
      messages: [
        {
          role: "user",
          content: "Hello",
        },
        {
          role: "assistant",
          content: "Hi",
        },
        {
          role: "user",
          content: "👋",
        },
        { role: "assistant", content: "👋" },
        {
          role: "user",
          content: "Hello",
        },
        {
          role: "assistant",
          content: "Hi",
        },
        {
          role: "user",
          content: "👋",
        },
        { role: "assistant", content: "👋" },
        {
          role: "user",
          content: "Hello",
        },
        {
          role: "assistant",
          content: "Hi",
        },
        {
          role: "user",
          content: "👋",
        },
        { role: "assistant", content: "👋" },
        {
          role: "user",
          content: "Hello",
        },
        {
          role: "assistant",
          content: "Hi",
        },
        {
          role: "user",
          content: "👋",
          // change this to see different button colours
          compression_strength: "low",
        },
        { role: "assistant", content: "👋" },
      ],
    }),
  },

  parameters: {
    msw: {
      handlers: CHAT_HANDLERS,
    },
  },
};
