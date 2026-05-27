import { beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "../../../utils/test-utils";
import type {
  EventMessage,
  EventSubkind,
} from "../../../services/refact/types";
import { EventLog } from "./EventLog";

function makeEvent(
  messageId: string,
  subkind: EventSubkind,
  content: string,
): EventMessage {
  return {
    role: "event",
    message_id: messageId,
    content,
    subkind,
    source: "test.source",
    payload: {
      created_at_ms: 1_700_000_000_000,
      messageId,
      nested: { ok: true },
    },
  };
}

const modeSwitchEvent = makeEvent("event-1", "mode_switch", "Mode switched");
const toolDecisionEvent = makeEvent(
  "event-2",
  "tool_decision",
  "Tool accepted",
);
const processEvent = makeEvent(
  "event-3",
  "process_completed",
  "Process completed",
);

const events = [modeSwitchEvent, toolDecisionEvent, processEvent];

describe("EventLog", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("renders nothing when events array is empty", () => {
    render(<EventLog events={[]} threadId="thread-empty" />);

    expect(screen.queryByTestId("event-log")).not.toBeInTheDocument();
  });

  it("renders disclosure closed by default with N events", () => {
    const { container } = render(
      <EventLog events={events} threadId="thread-default" />,
    );

    expect(screen.getByTestId("event-log")).toBeInTheDocument();
    expect(screen.getByText("Event log")).toBeInTheDocument();
    expect(screen.getByText("3 events")).toBeInTheDocument();
    expect(container.querySelector("details")).not.toHaveAttribute("open");
  });

  it("click to expand reveals all entries", () => {
    render(<EventLog events={events} threadId="thread-expand" />);

    fireEvent.click(screen.getByText("Event log"));

    expect(screen.getByText("Mode switched")).toBeInTheDocument();
    expect(screen.getByText("Tool accepted")).toBeInTheDocument();
    expect(screen.getByText("Process completed")).toBeInTheDocument();
    expect(screen.getAllByTestId("event-log-entry")).toHaveLength(3);
  });

  it("click a single entry expands its JSON payload", () => {
    render(<EventLog events={events} threadId="thread-json" />);

    fireEvent.click(screen.getByText("Event log"));
    fireEvent.click(screen.getByText("Mode switched"));

    expect(screen.getByTestId("event-log-json-event-1")).toHaveTextContent(
      '"messageId": "event-1"',
    );
    expect(
      screen.queryByTestId("event-log-json-event-2"),
    ).not.toBeInTheDocument();
  });

  it("filter chip toggle hides entries of that subkind", () => {
    render(<EventLog events={events} threadId="thread-filter" />);

    fireEvent.click(screen.getByText("Event log"));
    fireEvent.click(screen.getByLabelText(/mode_switch/));

    expect(screen.queryByText("Mode switched")).not.toBeInTheDocument();
    expect(screen.getByText("Tool accepted")).toBeInTheDocument();
    expect(screen.getByText("Process completed")).toBeInTheDocument();
  });

  it("localStorage persistence restores expanded and filter state", () => {
    const { unmount } = render(
      <EventLog events={events} threadId="thread-persist" />,
    );

    fireEvent.click(screen.getByText("Event log"));
    fireEvent.click(screen.getByLabelText(/tool_decision/));
    expect(screen.queryByText("Tool accepted")).not.toBeInTheDocument();
    unmount();

    const { container } = render(
      <EventLog events={events} threadId="thread-persist" />,
    );

    expect(container.querySelector("details")).toHaveAttribute("open");
    expect(screen.getByText("Mode switched")).toBeInTheDocument();
    expect(screen.queryByText("Tool accepted")).not.toBeInTheDocument();
    expect(screen.getByLabelText(/tool_decision/)).not.toBeChecked();
  });

  it("default state per thread is independent", () => {
    const { unmount } = render(
      <EventLog events={events} threadId="thread-opened" />,
    );

    fireEvent.click(screen.getByText("Event log"));
    unmount();

    const { container } = render(
      <EventLog events={events} threadId="thread-fresh" />,
    );

    expect(container.querySelector("details")).not.toHaveAttribute("open");
  });

  it("filter chips persist independently per thread", () => {
    const { unmount } = render(
      <EventLog events={events} threadId="thread-filter-a" />,
    );

    fireEvent.click(screen.getByText("Event log"));
    fireEvent.click(screen.getByLabelText(/process_completed/));
    unmount();

    render(<EventLog events={events} threadId="thread-filter-b" />);
    fireEvent.click(screen.getByText("Event log"));

    expect(screen.getByText("Process completed")).toBeInTheDocument();
    const processFilter = screen.getByLabelText(/process_completed/);
    expect(processFilter).toBeChecked();
  });

  it("renders only present subkind filters", () => {
    render(<EventLog events={[modeSwitchEvent]} threadId="thread-present" />);

    fireEvent.click(screen.getByText("Event log"));
    const eventLog = screen.getByTestId("event-log");

    expect(within(eventLog).getByLabelText(/mode_switch/)).toBeInTheDocument();
    expect(
      within(eventLog).queryByLabelText(/tool_decision/),
    ).not.toBeInTheDocument();
  });
});
