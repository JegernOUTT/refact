import type { Meta, StoryObj } from "@storybook/react";
import { ChatStoryHarness } from "../../../__stories__/ChatStoryHarness";
import { EventRow } from "./EventRow";
import { EVENT_SUBKINDS } from "./eventSubkind";

const meta = {
  title: "Chat/Event rows",
  component: EventRow,
  decorators: [
    (Story) => (
      <ChatStoryHarness>
        <Story />
      </ChatStoryHarness>
    ),
  ],
  args: {
    run: "single",
    event: {
      role: "event",
      subkind: "system_notice",
      source: "chat.session",
      content: "Runtime synchronized",
    },
  },
} satisfies Meta<typeof EventRow>;
export default meta;
type Story = StoryObj<typeof meta>;
export const AllSubkinds: Story = {
  render: () => (
    <>
      {EVENT_SUBKINDS.map((subkind, index) => (
        <EventRow
          key={subkind}
          run={
            index === 0
              ? "start"
              : index === EVENT_SUBKINDS.length - 1
                ? "end"
                : "middle"
          }
          event={{
            role: "event",
            subkind,
            source: "chat.session",
            content:
              "An event with a long summary that truncates without widening the transcript",
            payload: { seq: index, timestamp: "2025-01-01T12:34:56Z" },
          }}
        />
      ))}
    </>
  ),
};
export const Failure: Story = {
  args: {
    event: {
      role: "event",
      subkind: "process_completed",
      source: "exec.registry",
      content: "Tests failed",
      payload: {
        process_id: "process-1",
        short_description: "npm test",
        exit_code: 1,
        duration_ms: 1234,
      },
    },
  },
};
export const Unknown: Story = {
  args: {
    event: {
      role: "event",
      subkind: "future_kind",
      source: "engine",
      content: "Forward compatible event",
    },
  },
};
