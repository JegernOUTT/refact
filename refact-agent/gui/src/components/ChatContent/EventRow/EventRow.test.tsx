import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "../../../utils/test-utils";
import userEvent from "@testing-library/user-event";
import { EventRow } from "./EventRow";
import {
  EVENT_SUBKINDS,
  eventMessageTone,
  eventSubkindLabel,
} from "./eventSubkind";
import type { EventMessage } from "../../../services/refact/types";

const event = (
  subkind: string,
  payload: Record<string, unknown> = {},
): EventMessage => ({
  role: "event",
  subkind,
  source: "chat.session",
  content: "An event occurred",
  payload,
});

describe("EventRow", () => {
  it.each(EVENT_SUBKINDS)(
    "renders %s with its tone and typed expansion",
    (subkind) => {
      const message = event(subkind);
      render(<EventRow event={message} run="single" />);
      const row = screen.getByTestId("event-row");
      expect(row.getAttribute("data-tone")).toBe(eventMessageTone(message));
      expect(screen.getByText(eventSubkindLabel(subkind))).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { expanded: false }));
      expect(row.getAttribute("data-expanded")).toBe("true");
      expect(screen.getByTestId("event-row-detail")).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { expanded: true }));
      expect(screen.queryByTestId("event-row-detail")).toBeNull();
    },
  );

  it.each(["single", "start", "middle", "end"] as const)(
    "exposes %s rail position",
    (run) => {
      render(<EventRow event={event("future_kind")} run={run} />);
      expect(screen.getByTestId("event-row").getAttribute("data-run")).toBe(
        run,
      );
      expect(screen.getByTestId("event-row").getAttribute("data-tone")).toBe(
        "muted",
      );
      expect(screen.queryByText("--:--:--")).toBeNull();
    },
  );

  it("renders an unknown subkind with a humanized label and generic detail", () => {
    render(<EventRow event={event("future_kind")} run="single" />);
    expect(screen.getByText("Future kind")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { expanded: false }));
    expect(screen.getByText("Source")).toBeTruthy();
    expect(screen.getByText("Kind")).toBeTruthy();
  });

  it("renders a derived timestamp and omits it when none exists", () => {
    const { unmount } = render(
      <EventRow
        event={event("tick", { timestamp: "2025-01-01T12:34:56" })}
        run="single"
      />,
    );
    expect(screen.getByText("12:34:56")).toBeTruthy();
    unmount();

    render(<EventRow event={event("tick")} run="single" />);
    expect(screen.queryByText(/^\d{2}:\d{2}:\d{2}$/)).toBeNull();
  });

  it("supports keyboard toggling and a collapsed payload", async () => {
    const user = userEvent.setup();
    render(
      <EventRow
        event={event("mode_switch", { from: "agent", to: "task_planner" })}
        run="single"
      />,
    );
    screen.getByRole("button").focus();
    await user.keyboard("{Enter}");
    const header = screen.getByRole("button");
    expect(header.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByTestId("event-row-detail").id).toBe(
      header.getAttribute("aria-controls"),
    );
    expect(
      screen.getByText("Payload").parentElement?.hasAttribute("open"),
    ).toBe(false);
    await user.keyboard(" ");
    expect(header.getAttribute("aria-expanded")).toBe("false");
  });

  it("opens the process output", () => {
    const open = vi.fn();
    render(
      <EventRow
        event={event("process_completed", { process_id: "p1" })}
        run="single"
        onOpenProcessOutput={open}
      />,
    );
    fireEvent.click(screen.getByRole("button"));
    fireEvent.click(screen.getByRole("button", { name: "Open output" }));
    expect(open).toHaveBeenCalledWith("p1");
  });

  it("opens the scheduler task", () => {
    const { store } = render(
      <EventRow
        event={event("cron_fire", { task_id: "task1" })}
        run="single"
      />,
    );
    fireEvent.click(screen.getByRole("button"));
    fireEvent.click(screen.getByRole("button", { name: "Open scheduler" }));
    expect(store.getState().pages.at(-1)).toEqual({
      name: "scheduler",
      taskId: "task1",
    });
  });
});
