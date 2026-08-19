import type { Meta, StoryObj } from "@storybook/react";
import { fn } from "@storybook/test";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import { makeChatThread } from "../../__stories__/chatStoryState";
import { ModeTransitionDialog } from "./ModeTransitionDialog";

const thread = makeChatThread({
  id: "mode-transition-story",
  mode: "agent",
  messages: [
    {
      role: "user",
      content: "Explain the authentication flow before we change modes.",
      message_id: "mode-story-user",
    },
  ],
});

const meta = {
  title: "Chat/Dialogs/ModeTransitionDialog",
  component: ModeTransitionDialog,
  args: {
    open: true,
    onOpenChange: fn(),
    chatId: thread.id,
    currentMode: "agent",
    targetMode: "ask",
    targetModeTitle: "Ask",
    targetModeDescription: "Quick answers without editing the project.",
  },
  decorators: [
    (Story) => (
      <ChatStoryHarness thread={thread}>
        <Story />
      </ChatStoryHarness>
    ),
  ],
} satisfies Meta<typeof ModeTransitionDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const SwitchMode: Story = {};

export const RestartCurrentMode: Story = {
  args: {
    targetMode: "agent",
    targetModeTitle: "Agent",
    targetModeDescription: "Restart Agent with a compact context summary.",
  },
};
