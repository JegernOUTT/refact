import type { Meta, StoryObj } from "@storybook/react";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import {
  CHROME_MESSAGES,
  DELEGATE_MESSAGES,
  ENGINE_ANALYSIS_MESSAGES,
  FINISH_MESSAGES,
  PATCH_WITH_DIFF_MESSAGES,
  SET_TASKS_MESSAGES,
  SLEEP_ASK_MESSAGES,
  SUBAGENT_MESSAGES,
} from "../../__fixtures__/toolCardsAgentic";
import { CHAT_STORY_MSW_HANDLERS } from "../../__stories__/chatStoryState";
import type { ChatMessages } from "../../services/refact";
import { ChatContent } from ".";

type AgenticToolCardsStoryProps = {
  messages: ChatMessages;
};

const AgenticToolCardsStory = ({ messages }: AgenticToolCardsStoryProps) => (
  <ChatStoryHarness messages={messages}>
    <ChatContent onRetry={() => ({})} onStopStreaming={() => ({})} />
  </ChatStoryHarness>
);

const meta = {
  title: "Tool Cards/Agentic & Analysis",
  component: AgenticToolCardsStory,
  parameters: {
    msw: {
      handlers: [...CHAT_STORY_MSW_HANDLERS],
    },
  },
} satisfies Meta<typeof AgenticToolCardsStory>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Subagent: Story = {
  args: { messages: SUBAGENT_MESSAGES },
};

export const Delegate: Story = {
  args: { messages: DELEGATE_MESSAGES },
};

export const SetTasks: Story = {
  args: { messages: SET_TASKS_MESSAGES },
};

export const Finish: Story = {
  args: { messages: FINISH_MESSAGES },
};

export const SleepAsk: Story = {
  args: { messages: SLEEP_ASK_MESSAGES },
};

export const PatchWithDiff: Story = {
  args: { messages: PATCH_WITH_DIFF_MESSAGES },
};

export const Chrome: Story = {
  args: { messages: CHROME_MESSAGES },
};

export const EngineAnalysis: Story = {
  args: { messages: ENGINE_ANALYSIS_MESSAGES },
};
