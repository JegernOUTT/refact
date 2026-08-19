import type { Meta, StoryObj } from "@storybook/react";
import { within, userEvent } from "@storybook/testing-library";
import { http, HttpResponse } from "msw";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import { makeChatThread } from "../../__stories__/chatStoryState";
import { ThreadInfoButton } from "./ThreadInfoButton";

const thread = makeChatThread({
  id: "thread-info-story",
  messages: [
    {
      role: "user",
      content: "Show me where this thread is stored.",
      message_id: "thread-info-user",
    },
  ],
});

const meta = {
  title: "Chat/Composer/ThreadInfoButton",
  component: ThreadInfoButton,
  args: { chatId: thread.id },
  decorators: [
    (Story) => (
      <ChatStoryHarness thread={thread}>
        <Story />
      </ChatStoryHarness>
    ),
  ],
  parameters: {
    msw: {
      handlers: [
        http.get("*/v1/trajectories/:id/path", ({ params }) =>
          HttpResponse.json({
            path: `/workspace/.refact/chats/${String(params.id)}.json`,
          }),
        ),
      ],
    },
  },
} satisfies Meta<typeof ThreadInfoButton>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Open: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "Thread info" }));
  },
};
