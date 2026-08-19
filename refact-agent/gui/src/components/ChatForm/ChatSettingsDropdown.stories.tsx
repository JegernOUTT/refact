import type { Meta, StoryObj } from "@storybook/react";
import { within, userEvent } from "@storybook/testing-library";
import { goodCaps, goodChatModes, goodPing } from "../../__fixtures__/msw";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import { makeChatThread } from "../../__stories__/chatStoryState";
import { ChatSettingsDropdown } from "./ChatSettingsDropdown";

const thread = makeChatThread({ model: "openai/gpt-4o" });

const meta = {
  title: "Chat/Composer/ChatSettingsDropdown",
  component: ChatSettingsDropdown,
  decorators: [
    (Story) => (
      <ChatStoryHarness thread={thread}>
        <Story />
      </ChatStoryHarness>
    ),
  ],
  parameters: {
    msw: { handlers: [goodCaps, goodPing, goodChatModes] },
  },
} satisfies Meta<typeof ChatSettingsDropdown>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Open: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      await canvas.findByRole(
        "button",
        { name: /gpt-4o/i },
        { timeout: 10000 },
      ),
    );
  },
};
