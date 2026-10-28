import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "../../../utils/test-utils";
import { EventRow } from "./EventRow";
import { EVENT_SUBKINDS, eventMessageTone } from "./eventSubkind";
import type { EventMessage } from "../../../services/refact/types";

const event = (
  subkind: string,
  payload: Record<string, unknown> = {},
  overrides: Partial<EventMessage> = {},
): EventMessage => ({
  role: "event",
  subkind,
  source: "chat.session",
  content: "An event occurred",
  payload,
  ...overrides,
});

function toggle() {
  return screen.getByTestId("transcript-toggle");
}

describe("EventRow", () => {
  it.each(EVENT_SUBKINDS)("renders %s as one transcript item", (subkind) => {
    const message = event(subkind);
    render(<EventRow event={message} />);
    const row = screen.getByTestId("event-row");
    expect(row.getAttribute("data-transcript-item")).toBe("true");
    expect(row.getAttribute("data-tone")).toBe(eventMessageTone(message));
    expect(screen.getByTestId("transcript-target").textContent).toBe(
      "An event occurred",
    );
    expect(toggle().getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(toggle());
    expect(row.getAttribute("data-open")).toBe("true");
    expect(screen.getByTestId("transcript-facts")).toBeTruthy();
  });

  it("maps verbs from subkind and source", () => {
    const { unmount } = render(
      <EventRow event={event("tool_decision", { decision: "approve" })} />,
    );
    expect(screen.getByTestId("transcript-verb").textContent).toBe("Approval");
    unmount();
    render(
      <EventRow
        event={event(
          "system_notice",
          {},
          {
            source: "agents.message",
            content: "[message from parent]\nKeep going",
          },
        )}
      />,
    );
    expect(screen.getByTestId("transcript-verb").textContent).toBe(
      "Subagent note",
    );
    expect(screen.getByTestId("transcript-target").textContent).toBe(
      "Keep going",
    );
  });

  it("separates unmatched subagent finished events from spawn briefs", () => {
    const { unmount } = render(
      <EventRow
        event={event(
          "system_notice",
          { agent_id: "agent-1", status: "completed" },
          { source: "agents.spawn", content: "[background subagent finished]" },
        )}
      />,
    );
    expect(screen.getByTestId("transcript-verb").textContent).toBe(
      "Subagent finished",
    );
    unmount();
    render(
      <EventRow
        event={event(
          "system_notice",
          { config_name: "default" },
          { source: "agents.spawn", content: "Do the task" },
        )}
      />,
    );
    expect(screen.getByTestId("transcript-verb").textContent).toBe(
      "Subagent brief",
    );
  });

  it("labels subagent questions and unknown notices", () => {
    const { unmount } = render(
      <EventRow
        event={event(
          "system_notice",
          {},
          { content: "[subagent question] agent a1 asks: ok?" },
        )}
      />,
    );
    expect(screen.getByTestId("transcript-verb").textContent).toBe("Question");
    unmount();
    render(<EventRow event={event("future_kind")} />);
    expect(screen.getByTestId("transcript-verb").textContent).toBe("Notice");
  });

  it("hides the pill on success and shows it on failure", () => {
    const { unmount } = render(
      <EventRow event={event("process_completed", { exit_code: 0 })} />,
    );
    expect(screen.queryByTestId("transcript-status-pill")).toBeNull();
    unmount();
    render(<EventRow event={event("process_completed", { exit_code: 2 })} />);
    const pill = screen.getByTestId("transcript-status-pill");
    expect(pill.getAttribute("data-visible")).toBe("true");
    expect(pill.textContent).toBe("Failed");
  });

  it("renders a short clock in the meta and omits it when absent", () => {
    const { unmount } = render(
      <EventRow event={event("tick", { timestamp: "2025-01-01T12:34:56" })} />,
    );
    expect(screen.getByTestId("transcript-meta").textContent).toBe("12:34");
    unmount();
    render(<EventRow event={event("tick")} />);
    expect(screen.queryByTestId("transcript-meta")).toBeNull();
  });

  it("exposes the payload as raw", () => {
    render(
      <EventRow
        event={event("mode_switch", { from: "agent", to: "task_planner" })}
      />,
    );
    fireEvent.click(toggle());
    const raw = screen.getByTestId("transcript-raw");
    expect(within(raw).getByRole("button", { name: "Raw" })).toBeTruthy();
  });

  it("opens the process output", () => {
    const open = vi.fn();
    render(
      <EventRow
        event={event("process_completed", { process_id: "p1" })}
        onOpenProcessOutput={open}
      />,
    );
    fireEvent.click(toggle());
    fireEvent.click(screen.getByRole("button", { name: "Open output" }));
    expect(open).toHaveBeenCalledWith("p1");
  });

  it("opens the scheduler task", () => {
    const { store } = render(
      <EventRow event={event("cron_fire", { task_id: "task1" })} />,
    );
    fireEvent.click(toggle());
    fireEvent.click(screen.getByRole("button", { name: "Open scheduler" }));
    expect(store.getState().pages.at(-1)).toEqual({
      name: "scheduler",
      taskId: "task1",
    });
  });
});
