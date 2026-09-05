import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "../../../utils/test-utils";
import type {
  EventMessage,
  EventSubkind,
} from "../../../services/refact/types";
import { EventLog } from "./EventLog";
import { EVENT_SUBKINDS, eventSubkindLabel } from "./eventSubkind";

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
const planDeltaEvent = makeEvent("event-5", "plan_delta", "Plan updated");

const events = [modeSwitchEvent, toolDecisionEvent, processEvent];

function openLog(): void {
  fireEvent.click(screen.getByText("Event history"));
}

function pagesFromStore(store: RenderStore) {
  return store.getState().pages;
}

function storedFilters(threadId: string): EventSubkind[] {
  return JSON.parse(
    localStorage.getItem(`event-log-hidden-${threadId}`) ?? "[]",
  ) as EventSubkind[];
}

describe("EventLog", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("renders every known kind and newly introduced kinds by default", () => {
    const all = EVENT_SUBKINDS.map((kind, index) =>
      makeEvent(`all-${index}`, kind, `content-${kind}`),
    );
    all.push({
      ...makeEvent("future", "system_notice", "Future event payload"),
      subkind: "future_kind",
    });
    render(<EventLog events={all} threadId="all-kinds" />);
    openLog();
    expect(screen.getAllByTestId("event-log-entry")).toHaveLength(14);
    for (const kind of EVENT_SUBKINDS)
      expect(screen.getByLabelText(eventSubkindLabel(kind))).toBeChecked();
    expect(screen.getByLabelText("Future kind")).toBeChecked();
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
    expect(screen.getByText("Event history")).toBeInTheDocument();
    expect(screen.getByText("3 events")).toBeInTheDocument();
    expect(container.querySelector("details")).not.toHaveAttribute("open");
  });

  it("keeps native disclosure semantics for the event log summary", () => {
    const { container } = render(
      <EventLog events={events} threadId="thread-semantics" />,
    );

    const details = container.querySelector("details");
    const summary = container.querySelector("summary");

    expect(details).toBeInTheDocument();
    expect(summary).toHaveTextContent("Event history");

    openLog();

    expect(details).toHaveAttribute("open");
  });

  it("includes plan updates in the visible log", () => {
    render(
      <EventLog
        events={[modeSwitchEvent, planDeltaEvent]}
        threadId="thread-plan-delta"
      />,
    );

    openLog();

    expect(screen.getByText("2 events")).toBeInTheDocument();
    expect(screen.getByText("Mode switched")).toBeInTheDocument();
    expect(screen.getByText("Plan updated")).toBeInTheDocument();
    expect(screen.getByLabelText("Plan update")).toBeChecked();
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
    fireEvent.click(screen.getByLabelText(/Mode switch/));

    expect(screen.queryByText("Mode switched")).not.toBeInTheDocument();
    expect(screen.getByText("Tool accepted")).toBeInTheDocument();
    expect(screen.getByText("Process completed")).toBeInTheDocument();
    expect(storedFilters("thread-filter")).toEqual(["mode_switch"]);
  });

  it("keeps the disclosure visible when no events match active filters", () => {
    render(<EventLog events={[modeSwitchEvent]} threadId="thread-no-match" />);

    openLog();
    fireEvent.click(screen.getByLabelText(/Mode switch/));

    expect(screen.getByTestId("event-log")).toBeInTheDocument();
    expect(screen.getByText("1 event")).toBeInTheDocument();
    expect(
      screen.getByText("All event types are hidden by filters."),
    ).toBeInTheDocument();
  });

  it("localStorage persistence restores expanded and filter state", () => {
    const { unmount } = render(
      <EventLog events={events} threadId="thread-persist" />,
    );

    openLog();
    fireEvent.click(screen.getByLabelText(/Tool decision/));
    expect(screen.queryByText("Tool accepted")).not.toBeInTheDocument();
    unmount();

    const { container } = render(
      <EventLog events={events} threadId="thread-persist" />,
    );

    expect(container.querySelector("details")).toHaveAttribute("open");
    expect(screen.getByText("Mode switched")).toBeInTheDocument();
    expect(screen.queryByText("Tool accepted")).not.toBeInTheDocument();
    expect(screen.getByLabelText(/Tool decision/)).not.toBeChecked();
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
    fireEvent.click(screen.getByLabelText(/Process finished/));
    unmount();

    render(<EventLog events={events} threadId="thread-filter-b" />);
    openLog();

    expect(screen.getByText("Process completed")).toBeInTheDocument();
    const processFilter = screen.getByLabelText(/Process finished/);
    expect(processFilter).toBeChecked();
  });

  it("renders only present subkind filters", () => {
    render(<EventLog events={[modeSwitchEvent]} threadId="thread-present" />);

    openLog();
    const eventLog = screen.getByTestId("event-log");

    expect(within(eventLog).getByLabelText(/Mode switch/)).toBeInTheDocument();
    expect(
      within(eventLog).queryByLabelText(/Tool decision/),
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
