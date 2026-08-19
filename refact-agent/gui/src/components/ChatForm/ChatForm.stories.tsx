import type { Meta, StoryObj } from "@storybook/react";
import { within } from "@storybook/testing-library";

import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import {
  emptyWorktrees,
  goodCaps,
  goodChatModes,
  goodPing,
  goodPrompts,
  goodTools,
  goodUser,
  goodVoiceStatus,
  chatLinks,
  noCommandPreview,
  noCompletions,
} from "../../__fixtures__/msw";
import type { ImageFile, QueuedItem } from "../../features/Chat/Thread/types";
import {
  makeChatThread,
  type ThreadRuntime,
} from "../../__stories__/chatStoryState";
import { ChatForm } from "./ChatForm";

const queuedItems = [
  {
    client_request_id: "queued-refactor",
    priority: false,
    command_type: "chat",
    preview: "Refactor the authentication middleware",
    content: "Refactor the authentication middleware and preserve its tests.",
  },
  {
    client_request_id: "queued-tests",
    priority: true,
    command_type: "chat",
    preview: "Add regression coverage for expired sessions",
    content: "Add regression coverage for expired sessions.",
  },
] satisfies QueuedItem[];

const attachedImages = [
  {
    name: "architecture.png",
    type: "image/png",
    content:
      "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxNjAiIGhlaWdodD0iOTAiPjxyZWN0IHdpZHRoPSIxNjAiIGhlaWdodD0iOTAiIGZpbGw9IiMyYjJkMzEiLz48dGV4dCB4PSIxOCIgeT0iNDgiIGZpbGw9IndoaXRlIiBmb250LXNpemU9IjE0Ij5BcmNoaXRlY3R1cmU8L3RleHQ+PC9zdmc+",
  },
  {
    name: "error-state.png",
    type: "image/png",
    content:
      "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxNjAiIGhlaWdodD0iOTAiPjxyZWN0IHdpZHRoPSIxNjAiIGhlaWdodD0iOTAiIGZpbGw9IiM3YjI0MjQiLz48dGV4dCB4PSIyOCIgeT0iNDgiIGZpbGw9IndoaXRlIiBmb250LXNpemU9IjE0Ij5FcnJvciBzdGF0ZTwvdGV4dD48L3N2Zz4=",
  },
] satisfies ImageFile[];

type ComposerStoryProps = {
  runtime?: Partial<ThreadRuntime>;
};

function ComposerStory({ runtime }: ComposerStoryProps) {
  return (
    <ChatStoryHarness
      thread={makeChatThread({ mode: "agent", tool_use: "agent" })}
      runtime={runtime}
    >
      <ChatForm onSubmit={() => undefined} onClose={() => undefined} />
    </ChatStoryHarness>
  );
}

const meta = {
  title: "Chat Form",
  component: ComposerStory,
  parameters: {
    msw: {
      handlers: [
        emptyWorktrees,
        goodCaps,
        goodChatModes,
        goodPing,
        goodPrompts,
        goodUser,
        goodVoiceStatus,
        chatLinks,
        goodTools,
        noCompletions,
        noCommandPreview,
      ],
    },
  },
} satisfies Meta<typeof ComposerStory>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Primary: Story = {};

export const WithQueuedMessages: Story = {
  args: {
    runtime: { queued_items: queuedItems },
  },
};

export const WithAttachedImages: Story = {
  args: {
    runtime: { attached_images: attachedImages },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const textarea =
      await canvas.findByTestId<HTMLTextAreaElement>("chat-form-textarea");
    textarea.focus();
  },
};

export const Streaming: Story = {
  args: {
    runtime: { streaming: true },
  },
};
