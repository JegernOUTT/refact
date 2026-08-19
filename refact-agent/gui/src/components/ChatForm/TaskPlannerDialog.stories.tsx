import type { Meta, StoryObj } from "@storybook/react";
import { fn } from "@storybook/test";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import { makeChatThread } from "../../__stories__/chatStoryState";
import { TaskPlannerDialog } from "./TaskPlannerDialog";

const thread = makeChatThread({
  id: "task-planner-dialog-story",
  mode: "agent",
  messages: [
    {
      role: "user",
      content: "Plan the migration of our settings UI to the new components.",
      message_id: "planner-story-user",
    },
  ],
});

const meta = {
  title: "Chat/Dialogs/TaskPlannerDialog",
  component: TaskPlannerDialog,
  args: {
    sourceChatId: thread.id,
    open: true,
    onOpenChange: fn(),
    targetModeDescription:
      "Create a structured task and coordinate implementation chats.",
  },
  decorators: [
    (Story) => (
      <ChatStoryHarness thread={thread}>
        <Story />
      </ChatStoryHarness>
    ),
  ],
} satisfies Meta<typeof TaskPlannerDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const CreateNewTask: Story = {};

export const AddPlannerToTask: Story = {
  args: {
    taskId: "task-settings-migration",
  },
};
