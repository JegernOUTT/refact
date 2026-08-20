/* eslint-disable @typescript-eslint/no-non-null-assertion */
import type { Meta, StoryObj } from "@storybook/react";
import { ChatContent } from ".";
import { MarkdownMessage } from "../../__fixtures__/markdown";
import type { ChatMessages } from "../../services/refact";
import type { ChatThread } from "../../features/Chat/Thread";
import {
  CHAT_FUNCTIONS_MESSAGES,
  CHAT_WITH_DIFF_ACTIONS,
  CHAT_WITH_DIFFS,
  FROG_CHAT,
  LARGE_DIFF,
  CHAT_WITH_MULTI_MODAL_IMAGES,
  CHAT_CONFIG_THREAD,
  CHAT_WITH_TEXTDOC,
  MARKDOWN_ISSUE,
} from "../../__fixtures__";
import { userEvent, within } from "@storybook/testing-library";
import { chatLinks } from "../../__fixtures__/msw";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import { CHAT_STORY_MSW_HANDLERS } from "../../__stories__/chatStoryState";
import { makeChatThread } from "../../__stories__/chatStoryState";
import type { QueuedItem } from "../../features/Chat/Thread/types";

type ChatContentStoryProps = {
  messages?: ChatMessages;
  thread?: ChatThread;
};

// Shared harness (store + Theme + AbortControllers + ChatThreadProvider) with
// no story chrome: the transcript owns the whole preview viewport, exactly as
// it does inside the application shell.
const ChatContentStory = ({ messages, thread }: ChatContentStoryProps) => (
  <ChatStoryHarness thread={thread} messages={messages ?? []}>
    <ChatContent onRetry={() => ({})} onStopStreaming={() => ({})} />
  </ChatStoryHarness>
);

const meta = {
  title: "Chat Content",
  component: ChatContentStory,
  args: {
    messages: [],
  },
  parameters: {
    layout: "fullscreen",
    msw: {
      handlers: CHAT_STORY_MSW_HANDLERS,
    },
  },
} satisfies Meta<typeof ChatContentStory>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Primary: Story = {};

export const WithFunctions: Story = {
  args: {
    ...meta.args,
    messages: CHAT_FUNCTIONS_MESSAGES,
  },
};

export const Notes: Story = {
  args: {
    messages: FROG_CHAT.messages,
  },
};

export const WithDiffs: Story = {
  args: {
    messages: CHAT_WITH_DIFFS,
  },
};

export const WithDiffActions: Story = {
  args: {
    messages: CHAT_WITH_DIFF_ACTIONS.messages,
  },
};

export const LargeDiff: Story = {
  args: {
    messages: LARGE_DIFF.messages,
  },
};

export const Empty: Story = {
  args: {
    ...meta.args,
  },
};

export const AssistantMarkdown: Story = {
  args: {
    ...meta.args,
    messages: [{ role: "assistant", content: MarkdownMessage }],
  },
};

// The multimodal fixture pairs a `chrome`-named call (routed to <ChromeTool>,
// whose screenshots live inside the collapsible ToolCard body) with a
// non-chrome `screenshot` call (routed to <MultiModalToolContent>, whose image
// strip renders outside the collapsible and is visible while collapsed). Both
// results carry the real 480x270 PNG fixture.
const TOOL_IMAGE_MESSAGES = CHAT_WITH_MULTI_MODAL_IMAGES.slice(0, 3);

export const ToolImages: Story = {
  args: {
    ...meta.args,
    messages: TOOL_IMAGE_MESSAGES,
  },
  // The chrome tool card renders its screenshots inside the collapsible body,
  // so expand every collapsed tool card to cover the in-body images too.
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const buttons = await canvas.findAllByRole("button");
    const collapsed = buttons.filter(
      (button) => button.getAttribute("aria-expanded") === "false",
    );
    for (const toggle of collapsed) {
      await userEvent.click(toggle);
    }
  },
};

export const MultiModal: Story = {
  args: {
    messages: CHAT_WITH_MULTI_MODAL_IMAGES,
  },
};

export const IntegrationChat: Story = {
  args: {
    thread:
      CHAT_CONFIG_THREAD.threads[CHAT_CONFIG_THREAD.current_thread_id]!.thread,
  },
  parameters: {
    msw: {
      handlers: [...CHAT_STORY_MSW_HANDLERS, chatLinks],
    },
  },
};

export const TextDoc: Story = {
  args: {
    thread: CHAT_WITH_TEXTDOC,
  },
};

export const MarkdownIssue: Story = {
  args: {
    thread: MARKDOWN_ISSUE,
  },
};

export const ToolWaiting: Story = {
  args: {
    thread: {
      ...MARKDOWN_ISSUE,
      messages: [
        { role: "user", content: "call a tool and wait" },
        {
          role: "assistant",
          content: "",
          tool_calls: [
            {
              id: "toolu_01JbWarAwzjMyV6azDkd5skX",
              function: {
                arguments: '{"use_ast": true}',
                name: "tree",
              },
              type: "function",
              index: 0,
            },
          ],
        },
      ],
    },
  },
};

const queuedItems = [
  {
    client_request_id: "queued-documentation",
    priority: false,
    command_type: "chat",
    preview: "Document the new composer states",
    content: "Document the new composer states and include screenshots.",
  },
  {
    client_request_id: "queued-review",
    priority: true,
    command_type: "chat",
    preview: "Review the queued-message behavior",
    content: "Review queued-message behavior before publishing.",
  },
] satisfies QueuedItem[];

export const WithQueuedMessages: Story = {
  render: () => (
    <ChatStoryHarness
      thread={makeChatThread({
        messages: [
          { role: "user", content: "Prepare the composer documentation." },
          {
            role: "assistant",
            content: "I’ll prepare the documentation and then review it.",
          },
        ],
      })}
      runtime={{ queued_items: queuedItems, streaming: true }}
    >
      <ChatContent
        onRetry={() => undefined}
        onStopStreaming={() => undefined}
      />
    </ChatStoryHarness>
  ),
};
