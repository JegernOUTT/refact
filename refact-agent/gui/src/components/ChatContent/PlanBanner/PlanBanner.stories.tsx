import type { Meta, StoryObj } from "@storybook/react";

import { ChatStoryHarness } from "../../../__stories__/ChatStoryHarness";
import {
  makeChatThread,
  STORY_CHAT_ID,
} from "../../../__stories__/chatStoryState";
import type {
  ChatMessages,
  EventMessage,
  PlanMessage,
} from "../../../services/refact/types";
import { PlanBanner } from "./PlanBanner";

const basePlan: PlanMessage = {
  role: "plan",
  message_id: "plan-v1",
  content:
    "## Composer story plan\n\n1. Inspect the runtime types.\n2. Add fixtures for each composer state.\n3. Review the resulting stories.",
  extra: {
    plan: {
      mode: "agent",
      version: 1,
      created_at_ms: Date.now() - 8 * 60_000,
    },
  },
};

const planDeltas = [
  {
    role: "event",
    message_id: "plan-delta-1",
    content:
      "### Progress update\n\nRuntime and attachment types are verified.",
    subkind: "plan_delta",
    source: "tool.update_plan",
    payload: { seq: 1, status: "completed", step: "Inspect runtime types" },
    extra: {
      event: {
        subkind: "plan_delta",
        source: "tool.update_plan",
        payload: { seq: 1, status: "completed", step: "Inspect runtime types" },
      },
    },
  },
  {
    role: "event",
    message_id: "plan-delta-2",
    content:
      "### Current work\n\nAdding visual states for queued messages and images.",
    subkind: "plan_delta",
    source: "tool.update_plan",
    payload: { seq: 2, status: "in_progress", step: "Add story fixtures" },
    extra: {
      event: {
        subkind: "plan_delta",
        source: "tool.update_plan",
        payload: { seq: 2, status: "in_progress", step: "Add story fixtures" },
      },
    },
  },
] satisfies EventMessage[];

type PlanBannerStoryProps = {
  messages: ChatMessages;
};

function PlanBannerStory({ messages }: PlanBannerStoryProps) {
  const thread = makeChatThread({ messages, mode: "agent", tool_use: "agent" });
  return (
    <ChatStoryHarness thread={thread} height="520px">
      <PlanBanner threadId={STORY_CHAT_ID} />
    </ChatStoryHarness>
  );
}

const meta = {
  title: "Chat Content/Plan Banner",
  component: PlanBannerStory,
  args: { messages: [basePlan] },
} satisfies Meta<typeof PlanBannerStory>;

export default meta;
type Story = StoryObj<typeof meta>;

export const PlanV1: Story = {};

export const PlanWithDeltas: Story = {
  args: { messages: [basePlan, ...planDeltas] },
};
