import type { Meta, StoryObj } from "@storybook/react";
import { within, userEvent } from "@storybook/testing-library";
import { http, HttpResponse } from "msw";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import type { TrajectoryMeta } from "../../services/refact/trajectories";
import { TrajectoryButton } from "./TrajectoryButton";

const trajectories: TrajectoryMeta[] = [
  {
    id: "trajectory-story",
    title: "Composer popover improvements",
    created_at: "2025-01-15T10:00:00Z",
    updated_at: "2025-01-15T10:20:00Z",
    model: "openai/gpt-4o",
    mode: "agent",
    message_count: 8,
    total_lines_added: 42,
    total_lines_removed: 7,
    tasks_total: 3,
    tasks_done: 2,
    tasks_failed: 0,
  },
];

const meta = {
  title: "Chat/Composer/TrajectoryButton",
  component: TrajectoryButton,
  decorators: [
    (Story) => (
      <ChatStoryHarness>
        <Story />
      </ChatStoryHarness>
    ),
  ],
  parameters: {
    msw: {
      handlers: [
        http.get("*/v1/trajectories", () =>
          HttpResponse.json({
            items: trajectories,
            next_cursor: null,
            has_more: false,
            total_count: trajectories.length,
          }),
        ),
        http.get("*/v1/trajectories/all", () =>
          HttpResponse.json(trajectories),
        ),
      ],
    },
  },
  // SB8 no longer auto-creates implicit action args, and the popover reports
  // open state during render/play (ImplicitActionsDuringRendering).
  args: {
    onOpenChange: () => undefined,
  },
} satisfies Meta<typeof TrajectoryButton>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Trigger: Story = {};

export const OpenPopover: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      canvas.getByRole("button", { name: "Compress or Handoff" }),
    );
  },
};
