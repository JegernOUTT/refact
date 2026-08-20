import type { Meta, StoryObj } from "@storybook/react";
import { fn } from "@storybook/test";

import { goodCaps, goodChatModes, goodPing } from "../../__fixtures__/msw";
import { FIXTURE_IMAGE_DATA_URI } from "../../__fixtures__/images";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import type { UserMessage } from "../../services/refact";
import { RetryForm } from "./RetryForm";

const withImages = [
  { type: "text", text: "Explain why this screenshot fails in CI" },
  {
    type: "image_url",
    image_url: {
      // Real 480x270 fixture: a 1x1 gif renders as an invisible thumbnail and
      // hides every attachment layout regression.
      url: FIXTURE_IMAGE_DATA_URI,
    },
  },
] satisfies UserMessage["content"];

const meta = {
  title: "Chat Form/RetryForm",
  component: RetryForm,
  decorators: [
    (Story) => (
      <ChatStoryHarness>
        <Story />
      </ChatStoryHarness>
    ),
  ],
  parameters: {
    msw: {
      handlers: [goodCaps, goodPing, goodChatModes],
    },
  },
  args: {
    onSubmit: fn(),
    onClose: fn(),
  },
} satisfies Meta<typeof RetryForm>;

export default meta;
type Story = StoryObj<typeof meta>;

export const TextOnly: Story = {
  args: {
    value: "Fix the flaky test in CI",
  },
};

export const WithImages: Story = {
  args: {
    value: withImages,
  },
};
