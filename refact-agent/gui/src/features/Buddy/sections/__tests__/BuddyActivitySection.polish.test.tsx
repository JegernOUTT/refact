import { describe, expect, it, vi } from "vitest";
import { render, screen } from "../../../../utils/test-utils";
import { BuddyActivityPanel } from "../../BuddyActivityPanel";
import { BuddyRecentErrorsPanel } from "../../BuddyRecentErrorsPanel";
import type { BuddyRuntimeEvent } from "../../types";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

describe("buddy UI polish", () => {
  it("non_clickable_activity_rows_are_not_tabbable_buttons", () => {
    render(
      <BuddyActivityPanel
        activities={[
          {
            icon: "📝",
            title: "Read-only activity",
            description: "No chat to open",
            timestamp: "2024-01-01T00:00:00Z",
            activity_type: "buddy_memory_garden",
            chat_id: null,
          },
        ]}
      />,
      { preloadedState: CONFIG_STATE },
    );

    const row =
      screen.getByText("Read-only activity").parentElement?.parentElement;
    expect(row).toBeInTheDocument();
    expect(row).not.toHaveAttribute("tabindex", "0");
    expect(row).not.toHaveAttribute("role");
    expect(
      screen.queryByRole("button", { name: /read-only activity/i }),
    ).not.toBeInTheDocument();
  });

  it("invalid_buddy_timestamps_render_empty_text", () => {
    const recentError: BuddyRuntimeEvent = {
      id: "runtime-invalid-time",
      signal_type: "chat_error",
      title: "Runtime error",
      source: "test",
      status: "failed",
      priority: "high",
      created_at: "not-a-date",
    };

    render(
      <>
        <BuddyActivityPanel
          activities={[
            {
              icon: "📝",
              title: "Invalid activity time",
              description: "Invalid time should be omitted",
              timestamp: "not-a-date",
              activity_type: "buddy_memory_garden",
              chat_id: null,
            },
          ]}
        />
        <BuddyRecentErrorsPanel
          recentErrors={[recentError]}
          onInvestigate={vi.fn()}
          onDismiss={vi.fn()}
        />
      </>,
      { preloadedState: CONFIG_STATE },
    );

    expect(screen.getByText("Invalid activity time")).toBeInTheDocument();
    expect(screen.getByText("Runtime error")).toBeInTheDocument();
    expect(screen.queryByText("Invalid Date")).not.toBeInTheDocument();
  });
});
