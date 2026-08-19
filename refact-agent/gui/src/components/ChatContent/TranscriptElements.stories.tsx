import type { Meta, StoryObj } from "@storybook/react";
import {
  COMPRESSED_USER_MESSAGE,
  COMPRESSION_REPORT,
  CONTEXT_FILES,
  ERROR_DISPLAY,
  EVENT_MESSAGES,
  LONG_USER_MESSAGE,
  REASONING_ASSISTANT,
  SKILL_CARDS,
  THINKING_BLOCKS_ASSISTANT,
  USER_WITH_IMAGES,
} from "../../__fixtures__/transcriptElements";
import {
  goodCaps,
  goodPing,
  goodPrompts,
  goodUser,
  noCommandPreview,
  noCompletions,
  noTools,
} from "../../__fixtures__/msw";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import type { ChatMessages } from "../../services/refact";
import { ChatContent } from ".";

type TranscriptStoryProps = {
  messages: ChatMessages;
};

const TranscriptStory = ({ messages }: TranscriptStoryProps) => (
  <ChatStoryHarness messages={messages}>
    <ChatContent
      onRetry={() => ({})}
      onStopStreaming={() => ({})}
      onRetryGeneration={() => ({})}
    />
  </ChatStoryHarness>
);

const meta = {
  title: "Chat Transcript/Elements",
  component: TranscriptStory,
  parameters: {
    layout: "fullscreen",
    msw: {
      handlers: [
        goodPing,
        goodCaps,
        goodPrompts,
        goodUser,
        noTools,
        noCompletions,
        noCommandPreview,
      ],
    },
  },
} satisfies Meta<typeof TranscriptStory>;

export default meta;
type Story = StoryObj<typeof meta>;

export const LongUserMessage: Story = {
  args: { messages: LONG_USER_MESSAGE },
  parameters: {
    docs: {
      description: {
        story:
          "A user turn over both clamp thresholds, showing the collapsed transcript and its Show more control.",
      },
    },
  },
};

export const CompressedUserMessage: Story = {
  args: { messages: COMPRESSED_USER_MESSAGE },
  parameters: {
    docs: {
      description: {
        story:
          "The leading compression marker selects the compacted-user-message presentation and bypasses the ordinary long-message clamp.",
      },
    },
  },
};

export const UserWithImages: Story = {
  args: { messages: USER_WITH_IMAGES },
};

export const Reasoning: Story = {
  args: { messages: REASONING_ASSISTANT },
  parameters: {
    docs: {
      description: {
        story:
          "Reasoning content includes a bold heading glued to the previous sentence so the production paragraph repair is visible.",
      },
    },
  },
};

export const ThinkingBlocks: Story = {
  args: { messages: THINKING_BLOCKS_ASSISTANT },
};

export const HiddenEvents: Story = {
  args: { messages: EVENT_MESSAGES },
  parameters: {
    docs: {
      description: {
        story:
          "Process completion, cron, and system-notice messages are intentionally absent from ordinary transcript turns; only the surrounding user and assistant messages are rendered by ChatContent.",
      },
    },
  },
  play: ({ canvasElement }) => {
    const hiddenEventText = [
      "Background index refresh completed successfully.",
      "Scheduled transcript maintenance fired.",
      "Internal transcript synchronization notice.",
    ];
    const transcriptText = canvasElement.textContent ?? "";
    for (const eventText of hiddenEventText) {
      if (transcriptText.includes(eventText)) {
        throw new Error(
          `Hidden event rendered as a transcript turn: ${eventText}`,
        );
      }
    }
  },
};

export const CompressionCards: Story = {
  args: { messages: COMPRESSION_REPORT },
  parameters: {
    docs: {
      description: {
        story:
          "Shows a metrics-rich compression report and a separate assistant segment-summary card produced from compression metadata.",
      },
    },
  },
};

export const ContextFilesAttachment: Story = {
  args: { messages: CONTEXT_FILES },
  parameters: {
    docs: {
      description: {
        story:
          "A context_file message keyed to the assistant's read-tool call is attached to that tool through buildDisplayItems.",
      },
    },
  },
};

export const SkillCards: Story = {
  args: { messages: SKILL_CARDS },
  parameters: {
    docs: {
      description: {
        story:
          "Current cd_instruction and plain_text protocol messages produce the skill activation and skill report cards.",
      },
    },
  },
};

export const ErrorCard: Story = {
  args: { messages: ERROR_DISPLAY },
  parameters: {
    docs: {
      description: {
        story:
          "A typed error-role message with structured user guidance and retry status produces the transcript error card.",
      },
    },
  },
};
