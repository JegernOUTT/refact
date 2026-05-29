import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "../../../utils/test-utils";
import type {
  EventMessage,
  EventSubkind,
} from "../../../services/refact/types";
import { EventLog } from "./EventLog";

type RenderStore = ReturnType<typeof render>["store"];

function makeEvent(
  messageId: string,
  subkind: EventSubkind,
  content: string,
  payload: Record<string, unknown> = {},
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
      ...payload,
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
  { process_id: "exec-process-1" },
);
const cronEvent = makeEvent("event-4", "cron_fire", "Cron fired", {
  task_id: "task-1",
});

const events = [modeSwitchEvent, toolDecisionEvent, processEvent];

function openLog(): void {
  fireEvent.click(screen.getByText("Event log"));
}

function pagesFromStore(store: RenderStore) {
  return store.getState().pages;
}

function storedFilters(threadId: string): EventSubkind[] {
  return JSON.parse(
    localStorage.getItem(`event-log-filter-${threadId}`) ?? "[]",
  ) as EventSubkind[];
}

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

    openLog();

    expect(screen.getByText("Mode switched")).toBeInTheDocument();
    expect(screen.getByText("Tool accepted")).toBeInTheDocument();
    expect(screen.getByText("Process completed")).toBeInTheDocument();
    expect(screen.getAllByTestId("event-log-entry")).toHaveLength(3);
  });

  it("click a single entry expands its JSON payload", () => {
    render(<EventLog events={events} threadId="thread-json" />);

    openLog();
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

    openLog();
    fireEvent.click(screen.getByLabelText(/mode_switch/));

    expect(screen.queryByText("Mode switched")).not.toBeInTheDocument();
    expect(screen.getByText("Tool accepted")).toBeInTheDocument();
    expect(screen.getByText("Process completed")).toBeInTheDocument();
    expect(storedFilters("thread-filter")).toEqual([
      "tool_decision",
      "ide_callback",
      "process_completed",
      "cron_fire",
      "tick",
      "summarization_marker",
      "cancellation_note",
      "verifier_report",
      "system_notice",
    ]);
  });

  it("keeps the disclosure visible when no events match active filters", () => {
    render(<EventLog events={[modeSwitchEvent]} threadId="thread-no-match" />);

    openLog();
    fireEvent.click(screen.getByLabelText(/mode_switch/));

    expect(screen.getByTestId("event-log")).toBeInTheDocument();
    expect(screen.getByText("1 event")).toBeInTheDocument();
    expect(
      screen.getByText("All event subkinds are hidden by filters."),
    ).toBeInTheDocument();
  });

  it("localStorage persistence restores expanded and filter state", () => {
    const { unmount } = render(
      <EventLog events={events} threadId="thread-persist" />,
    );

    openLog();
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

    openLog();
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

    openLog();
    fireEvent.click(screen.getByLabelText(/process_completed/));
    unmount();

    render(<EventLog events={events} threadId="thread-filter-b" />);
    openLog();

    expect(screen.getByText("Process completed")).toBeInTheDocument();
    const processFilter = screen.getByLabelText(/process_completed/);
    expect(processFilter).toBeChecked();
  });

  it("renders only present subkind filters", () => {
    render(<EventLog events={[modeSwitchEvent]} threadId="thread-present" />);

    openLog();
    const eventLog = screen.getByTestId("event-log");

    expect(within(eventLog).getByLabelText(/mode_switch/)).toBeInTheDocument();
    expect(
      within(eventLog).queryByLabelText(/tool_decision/),
    ).not.toBeInTheDocument();
  });

  it("click on process_completed entry calls scroll handler with process_id", () => {
    const onProcessCompletedClick = vi.fn();
    render(
      <EventLog
        events={[processEvent]}
        threadId="thread-process-click"
        onProcessCompletedClick={onProcessCompletedClick}
      />,
    );

    openLog();
    fireEvent.click(screen.getByText("Process completed"));

    expect(onProcessCompletedClick).toHaveBeenCalledWith("exec-process-1");
  });

  it("click on cron_fire entry dispatches the Scheduler-open action", () => {
    const { store } = render(
      <EventLog events={[cronEvent]} threadId="thread-cron-click" />,
    );

    openLog();
    fireEvent.click(screen.getByText("Cron fired"));

    expect(pagesFromStore(store).at(-1)).toEqual({
      name: "scheduler",
      taskId: "task-1",
    });
  });
});
