import React from "react";
import type { Meta, StoryObj } from "@storybook/react";
import {
  CAT_TOOL_MESSAGES,
  GENERIC_FALLBACK_TOOL_MESSAGES,
  KNOWLEDGE_TOOL_MESSAGES,
  MANY_TOOLS_GROUPED_MESSAGES,
  MOVE_REMOVE_TOOL_MESSAGES,
  REGEX_SEARCH_TOOL_MESSAGES,
  SHELL_TOOL_MESSAGES,
  TREE_TOOL_MESSAGES,
  WEB_SEARCH_TOOL_MESSAGES,
  WEB_TOOL_MESSAGES,
} from "../../__fixtures__/toolCardsFileOps";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import { CHAT_STORY_MSW_HANDLERS } from "../../__stories__/chatStoryState";
import type { ChatMessages } from "../../services/refact";
import { ChatContent } from "./ChatContent";

type ToolCardsStoryProps = {
  messages: ChatMessages;
};

const ToolCardsStory: React.FC<ToolCardsStoryProps> = ({ messages }) => (
  <ChatStoryHarness messages={messages}>
    <ChatContent onRetry={() => ({})} onStopStreaming={() => ({})} />
  </ChatStoryHarness>
);

const meta = {
  title: "Tool Cards/File Ops & Basics",
  component: ToolCardsStory,
  parameters: {
    msw: {
      handlers: [...CHAT_STORY_MSW_HANDLERS],
    },
  },
} satisfies Meta<typeof ToolCardsStory>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Cat: Story = {
  args: { messages: CAT_TOOL_MESSAGES },
};

export const Tree: Story = {
  args: { messages: TREE_TOOL_MESSAGES },
};

export const RegexSearch: Story = {
  args: { messages: REGEX_SEARCH_TOOL_MESSAGES },
};

export const Shell: Story = {
  args: { messages: SHELL_TOOL_MESSAGES },
};

export const MoveRemove: Story = {
  args: { messages: MOVE_REMOVE_TOOL_MESSAGES },
};

export const Web: Story = {
  args: { messages: WEB_TOOL_MESSAGES },
};

export const WebSearch: Story = {
  args: { messages: WEB_SEARCH_TOOL_MESSAGES },
};

export const Knowledge: Story = {
  args: { messages: KNOWLEDGE_TOOL_MESSAGES },
};

export const GenericFallback: Story = {
  args: { messages: GENERIC_FALLBACK_TOOL_MESSAGES },
};

export const ManyToolsGrouped: Story = {
  args: { messages: MANY_TOOLS_GROUPED_MESSAGES },
};
