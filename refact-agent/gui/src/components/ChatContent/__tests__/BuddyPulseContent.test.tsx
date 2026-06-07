import { Theme } from "@radix-ui/themes";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { BuddyPulseContent } from "../BuddyPulseContent";
import type { BuddyPulsePayload } from "../../../services/refact/types";

const samplePayload: BuddyPulsePayload = {
  preferences: [
    {
      statement: "Prefer compact task reports",
      confidence: 0.91,
      last_updated: "2026-05-15T00:00:00.000Z",
    },
  ],
  lessons: [
    {
      title: "Run targeted checks",
      preview: "Use focused GUI checks for pulse UI changes.",
      tags: ["gui", "buddy"],
      updated: "2026-05-15T00:00:00.000Z",
    },
  ],
  friction: {
    top_error_types: [{ type: "lint", count: 2 }],
    stuck_tasks: 1,
  },
  recent_reports: [
    {
      workflow_id: "buddy_project_pulse",
      title: "Project pulse report",
      preview: "Summarized the latest memory and activity signals.",
      chat_id: "chat-1",
    },
  ],
  user_activity: {
    grouped: [{ type: "file_edit", count: 3 }],
    time_of_day_pattern: "Most activity happened in the morning.",
  },
  generated_at: new Date().toISOString(),
};

const renderPulse = () =>
  render(
    <Theme>
      <BuddyPulseContent rawExtra={{ buddy_pulse_payload: samplePayload }} />
    </Theme>,
  );

describe("BuddyPulseContent", () => {
  it("renders_collapsed_by_default", () => {
    renderPulse();

    expect(screen.getByRole("button", { name: /Project pulse/ })).toBeTruthy();
    expect(screen.queryByText(/Preferences/)).toBeNull();
  });

  it("expands_on_header_click", () => {
    renderPulse();

    fireEvent.click(screen.getByRole("button", { name: /Project pulse/ }));

    expect(screen.getByText(/Preferences/)).toBeTruthy();
  });

  it("renders_all_5_sections_when_expanded", () => {
    renderPulse();

    fireEvent.click(screen.getByRole("button", { name: /Project pulse/ }));

    expect(screen.getByText(/Preferences/)).toBeTruthy();
    expect(screen.getByText(/Lessons/)).toBeTruthy();
    expect(screen.getByText(/Friction/)).toBeTruthy();
    expect(screen.getByText(/Recent reports/)).toBeTruthy();
    expect(screen.getByText(/Activity \(24h\)/)).toBeTruthy();
  });

  it("returns_null_when_payload_missing", () => {
    const { container } = render(<BuddyPulseContent rawExtra={undefined} />);

    expect(container.firstChild).toBeNull();
  });
});
