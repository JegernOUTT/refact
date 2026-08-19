import type { Meta, StoryObj } from "@storybook/react";

import {
  emptyWorktrees,
  goodCaps,
  goodChatModes,
  goodVoiceStatus,
} from "../../__fixtures__/msw";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import { makeChatThread } from "../../__stories__/chatStoryState";
import type { TodoItem } from "../../features/Chat/Thread/types";
import type { ChatMessages, GoalSnapshot } from "../../services/refact/types";
import { TaskProgressWidget } from "./TaskProgressWidget";

const activeGoal: GoalSnapshot = {
  content:
    "## Ship the composer stories\n\nCover queued messages, attachments, streaming, and goal progress.",
  version: 3,
  active: true,
  status: "active",
  budget: {
    max_turns: 12,
    max_minutes: 45,
    max_tokens: 24_000,
    cooldown_ms: 1_500,
    no_progress_token_threshold: 300,
    no_progress_turns: 3,
  },
  progress: {
    turns_used: 5,
    tokens_used: 8_450,
    started_at_ms: Date.now() - 18 * 60_000,
    no_progress_turns: 0,
    last_nudge_at_ms: Date.now() - 4 * 60_000,
  },
  attempts: [
    {
      at_ms: Date.now() - 10 * 60_000,
      trigger: "checkpoint",
      verdict: "needs_work",
      gaps: ["Streaming state still needs visual coverage"],
      verifier_reply: "Add the streaming composer story before completion.",
    },
  ],
  events: [
    {
      at_ms: Date.now() - 16 * 60_000,
      kind: "goal_pursuit",
      text: "Started inspecting composer runtime selectors.",
    },
    {
      at_ms: Date.now() - 9 * 60_000,
      kind: "goal_pursuit",
      text: "Added queued-message and attachment fixtures.",
    },
    {
      at_ms: Date.now() - 3 * 60_000,
      kind: "nudge",
      text: "Requested final story coverage review.",
    },
  ],
  transferred_from: null,
  transferred_to: null,
};

const tasks = [
  {
    id: "inspect",
    content: "Inspect real component props",
    status: "completed",
  },
  {
    id: "fixtures",
    content: "Create realistic story fixtures",
    status: "in_progress",
  },
  { id: "review", content: "Review all visual states", status: "pending" },
] satisfies TodoItem[];

function taskMessages(items: TodoItem[]): ChatMessages {
  return [
    {
      role: "assistant",
      message_id: "tasks-assistant",
      content: "Tracking the Storybook work.",
      tool_calls: [
        {
          id: "tasks-call",
          index: 0,
          type: "function",
          function: {
            name: "tasks_set",
            arguments: JSON.stringify({ tasks: items }),
          },
        },
      ],
    },
    {
      role: "tool",
      message_id: "tasks-result",
      tool_call_id: "tasks-call",
      content: "Task list updated",
      tool_failed: false,
    },
  ];
}

type GoalWidgetStoryProps = {
  goal: GoalSnapshot;
  messages?: ChatMessages;
};

function GoalWidgetStory({ goal, messages = [] }: GoalWidgetStoryProps) {
  const thread = makeChatThread({
    title: "Composer story goal",
    mode: "task_agent",
    tool_use: "agent",
    goal,
    messages,
  });

  return (
    <ChatStoryHarness
      thread={thread}
      runtime={{ task_widget_expanded: true, task_goal_expanded: true }}
      height="720px"
    >
      <TaskProgressWidget />
    </ChatStoryHarness>
  );
}

const meta = {
  title: "Task Progress Widget",
  component: GoalWidgetStory,
  args: { goal: activeGoal },
  parameters: {
    msw: {
      handlers: [emptyWorktrees, goodCaps, goodChatModes, goodVoiceStatus],
    },
  },
} satisfies Meta<typeof GoalWidgetStory>;

export default meta;
type Story = StoryObj<typeof meta>;

export const ActiveGoal: Story = {};

export const PausedGoal: Story = {
  args: {
    goal: { ...activeGoal, active: false, status: "paused" },
  },
};

export const Verifying: Story = {
  args: {
    goal: { ...activeGoal, active: true, status: "verifying" },
  },
};

export const CompletedGoal: Story = {
  args: {
    goal: {
      ...activeGoal,
      active: false,
      status: "completed",
      progress: { ...activeGoal.progress, turns_used: 9, tokens_used: 18_720 },
      events: [
        ...activeGoal.events,
        {
          at_ms: Date.now() - 60_000,
          kind: "completed",
          text: "Verifier accepted the completed story coverage.",
        },
      ],
    },
  },
};

export const BudgetExhausted: Story = {
  args: {
    goal: {
      ...activeGoal,
      active: false,
      status: "budget_exhausted",
      budget: {
        ...activeGoal.budget,
        max_turns: 6,
        max_minutes: 20,
        max_tokens: 10_000,
      },
      progress: { ...activeGoal.progress, turns_used: 6, tokens_used: 10_000 },
    },
  },
};

export const Stopped: Story = {
  args: {
    goal: { ...activeGoal, active: false, status: "stopped" },
  },
};

export const NoProgress: Story = {
  args: {
    goal: {
      ...activeGoal,
      active: false,
      status: "no_progress",
      progress: { ...activeGoal.progress, no_progress_turns: 3 },
    },
  },
};

export const Transferred: Story = {
  args: {
    goal: {
      ...activeGoal,
      active: false,
      status: "transferred",
      transferred_to: "composer-follow-up-chat",
    },
  },
};

export const NoBudgetLimits: Story = {
  args: {
    goal: {
      ...activeGoal,
      budget: {
        cooldown_ms: 1_500,
        no_progress_token_threshold: 300,
      },
    },
  },
};

export const WithTasks: Story = {
  args: {
    goal: activeGoal,
    messages: taskMessages(tasks),
  },
};
