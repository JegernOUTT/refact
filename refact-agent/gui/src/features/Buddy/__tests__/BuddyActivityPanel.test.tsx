import { describe, expect, it, vi } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { BuddyActivityPanel } from "../BuddyActivityPanel";
import type { BuddyActivityEntry } from "../types";

const activity = (
  index: number,
  overrides: Partial<BuddyActivityEntry> = {},
): BuddyActivityEntry => ({
  icon: "⚙️",
  title: `Activity ${index}`,
  description: "Completed a workflow task",
  timestamp: "2026-01-01T00:00:00Z",
  activity_type: `buddy_workflow_${index}`,
  chat_id: null,
  ...overrides,
});

describe("BuddyActivityPanel", () => {
  it("provides a capped semantic list and uses buttons only for openable chats", async () => {
    const onOpenChat = vi.fn();
    const activities = Array.from({ length: 22 }, (_, index) =>
      activity(index, {
        title: index < 2 ? "Repeated activity" : `Activity ${index}`,
        activity_type: index < 2 ? "buddy_repeat" : `buddy_workflow_${index}`,
        chat_id: index === 3 ? "chat-3" : null,
      }),
    );
    const { user } = render(
      <BuddyActivityPanel activities={activities} onOpenChat={onOpenChat} />,
    );

    expect(screen.getAllByRole("listitem")).toHaveLength(20);
    expect(screen.getByText("2 similar")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /^Activity 3.*Open Buddy chat$/ }),
    ).toBeInTheDocument();
    expect(screen.getAllByRole("listitem")).toHaveLength(20);

    await user.click(
      screen.getByRole("button", { name: /^Activity 3.*Open Buddy chat$/ }),
    );
    expect(onOpenChat).toHaveBeenCalledWith("chat-3", "Activity 3");
    await user.click(screen.getByRole("button", { name: "Show more" }));
    expect(screen.getAllByRole("listitem")).toHaveLength(21);
  });
});
