import type { Meta, StoryObj } from "@storybook/react";
import { within, userEvent } from "@storybook/testing-library";
import { fn } from "@storybook/test";
import { goodChatModes, goodPing } from "../../__fixtures__/msw";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import { ModeSelect } from "./ModeSelect";

const meta = {
  title: "Chat/Composer/ModeSelect",
  component: ModeSelect,
  args: {
    selectedMode: "agent",
    onModeChange: fn(),
  },
  decorators: [
    (Story) => (
      <ChatStoryHarness>
        <Story />
      </ChatStoryHarness>
    ),
  ],
  parameters: {
    msw: {
      handlers: [goodChatModes, goodPing],
    },
  },
} satisfies Meta<typeof ModeSelect>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Trigger: Story = {};

export const Open: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      await canvas.findByRole("button", { name: /Agent/ }, { timeout: 10000 }),
    );
  },
};
