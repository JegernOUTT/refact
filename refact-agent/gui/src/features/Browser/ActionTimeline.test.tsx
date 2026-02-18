/* eslint-disable @typescript-eslint/no-non-null-assertion */
import { describe, test, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Provider } from "react-redux";
import { configureStore } from "@reduxjs/toolkit";
import { ActionTimeline } from "./ActionTimeline";
import {
  browserSlice,
  setBrowserRuntime,
  addTimelineEntries,
  setTimelineFilterSource,
  setTimelineFilterType,
  clearTimeline,
  toggleTimelineOpen,
  type BrowserState,
  type BrowserRuntime,
  type TimelineEntry,
} from "./browserSlice";

const reducer = browserSlice.reducer;

function makeRuntime(overrides?: Partial<BrowserRuntime>): BrowserRuntime {
  return {
    runtime_id: "rt-1",
    connected: true,
    active_tab: null,
    url: null,
    title: null,
    tabs: [],
    latest_frame: null,
    picker_active: false,
    attach_screenshot_on_send: false,
    timeline: [],
    timeline_open: false,
    timeline_filter_source: "all",
    timeline_filter_type: null,
    notification: null,
    oversize_info: null,
    pending_toolbar_actions: [],
    ...overrides,
  };
}

function stateWith(
  chatId: string,
  runtime: BrowserRuntime,
): BrowserState {
  return { runtimes: { [chatId]: runtime }, browserUiOpen: {} };
}

const sampleEntries: TimelineEntry[] = [
  {
    timestamp: "2025-01-01T10:00:00Z",
    source: "user",
    type: "navigation",
    summary: "Navigated to https://example.com",
    details: { url: "https://example.com" },
  },
  {
    timestamp: "2025-01-01T10:00:01Z",
    source: "agent",
    type: "click",
    summary: "Clicked button #submit",
    details: { selector: "#submit", x: 100, y: 200 },
  },
  {
    timestamp: "2025-01-01T10:00:02Z",
    source: "user",
    type: "input",
    summary: "Typed text into search field",
  },
  {
    timestamp: "2025-01-01T10:00:03Z",
    source: "agent",
    type: "tool_call",
    summary: "Called chrome_screenshot",
    details: { tool: "chrome_screenshot" },
  },
];

function renderTimeline(entries: TimelineEntry[] = sampleEntries) {
  const store = configureStore({
    reducer: { browser: browserSlice.reducer },
    preloadedState: {
      browser: stateWith(
        "chat-1",
        makeRuntime({ timeline: entries, timeline_open: true }),
      ),
    },
  });

  return {
    store,
    ...render(
      <Provider store={store}>
        <ActionTimeline chatId="chat-1" />
      </Provider>,
    ),
  };
}

describe("ActionTimeline component", () => {
  test("renders timeline entries", () => {
    renderTimeline();

    const entries = screen.getAllByTestId("timeline-entry");
    expect(entries).toHaveLength(4);
  });

  test("shows empty state when no entries", () => {
    renderTimeline([]);

    expect(screen.getByText("No timeline events")).toBeInTheDocument();
  });

  test("displays entry summary text", () => {
    renderTimeline();

    expect(
      screen.getByText("Navigated to https://example.com"),
    ).toBeInTheDocument();
    expect(screen.getByText("Clicked button #submit")).toBeInTheDocument();
  });

  test("displays entry type badges", () => {
    renderTimeline();

    expect(screen.getAllByText("navigation").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("click").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("input").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("tool_call").length).toBeGreaterThanOrEqual(1);
  });

  test("shows user and agent source icons with aria labels", () => {
    renderTimeline();

    const userIcons = screen.getAllByLabelText("Source: user");
    const agentIcons = screen.getAllByLabelText("Source: agent");
    expect(userIcons).toHaveLength(2);
    expect(agentIcons).toHaveLength(2);
  });

  test("expands entry details on click", async () => {
    const user = userEvent.setup();
    renderTimeline();

    expect(screen.queryByTestId("entry-details")).not.toBeInTheDocument();

    const entries = screen.getAllByTestId("timeline-entry");
    await user.click(entries[0]);

    const details = screen.getByTestId("entry-details");
    expect(details).toBeInTheDocument();
    expect(details.textContent).toContain("https://example.com");
  });

  test("does not expand entry without details", async () => {
    const user = userEvent.setup();
    renderTimeline();

    const entries = screen.getAllByTestId("timeline-entry");
    await user.click(entries[2]);

    expect(screen.queryByTestId("entry-details")).not.toBeInTheDocument();
  });

  test("filters by source when clicking filter buttons", async () => {
    const user = userEvent.setup();
    renderTimeline();

    await user.click(screen.getByRole("button", { name: "User" }));
    expect(screen.getAllByTestId("timeline-entry")).toHaveLength(2);

    await user.click(screen.getByRole("button", { name: "Agent" }));
    expect(screen.getAllByTestId("timeline-entry")).toHaveLength(2);

    await user.click(screen.getByRole("button", { name: "All" }));
    expect(screen.getAllByTestId("timeline-entry")).toHaveLength(4);
  });

  test("filters by type when clicking type filter buttons", async () => {
    const user = userEvent.setup();
    renderTimeline();

    await user.click(screen.getByRole("button", { name: "navigation" }));
    expect(screen.getAllByTestId("timeline-entry")).toHaveLength(1);
    expect(
      screen.getByText("Navigated to https://example.com"),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "All types" }));
    expect(screen.getAllByTestId("timeline-entry")).toHaveLength(4);
  });
});

describe("browserSlice timeline reducers", () => {
  test("addTimelineEntries appends entries", () => {
    const initial = stateWith("chat-1", makeRuntime());
    const entries: TimelineEntry[] = [
      {
        timestamp: "2025-01-01T10:00:00Z",
        source: "user",
        type: "navigation",
        summary: "Nav",
      },
    ];
    const state = reducer(
      initial,
      addTimelineEntries({ chatId: "chat-1", entries }),
    );
    expect(state.runtimes["chat-1"]!.timeline).toHaveLength(1);
    expect(state.runtimes["chat-1"]!.timeline[0].summary).toBe("Nav");
  });

  test("addTimelineEntries appends to existing entries", () => {
    const existing: TimelineEntry[] = [
      {
        timestamp: "2025-01-01T09:00:00Z",
        source: "agent",
        type: "click",
        summary: "Existing",
      },
    ];
    const initial = stateWith(
      "chat-1",
      makeRuntime({ timeline: existing }),
    );
    const newEntries: TimelineEntry[] = [
      {
        timestamp: "2025-01-01T10:00:00Z",
        source: "user",
        type: "input",
        summary: "New",
      },
    ];
    const state = reducer(
      initial,
      addTimelineEntries({ chatId: "chat-1", entries: newEntries }),
    );
    expect(state.runtimes["chat-1"]!.timeline).toHaveLength(2);
    expect(state.runtimes["chat-1"]!.timeline[0].summary).toBe("Existing");
    expect(state.runtimes["chat-1"]!.timeline[1].summary).toBe("New");
  });

  test("addTimelineEntries does nothing for missing chatId", () => {
    const initial: BrowserState = { runtimes: {}, browserUiOpen: {} };
    const state = reducer(
      initial,
      addTimelineEntries({
        chatId: "missing",
        entries: [
          {
            timestamp: "t",
            source: "user",
            type: "nav",
            summary: "s",
          },
        ],
      }),
    );
    expect(state.runtimes).toEqual({});
  });

  test("clearTimeline empties timeline", () => {
    const initial = stateWith(
      "chat-1",
      makeRuntime({
        timeline: [
          {
            timestamp: "t",
            source: "user",
            type: "nav",
            summary: "s",
          },
        ],
      }),
    );
    const state = reducer(initial, clearTimeline({ chatId: "chat-1" }));
    expect(state.runtimes["chat-1"]!.timeline).toHaveLength(0);
  });

  test("toggleTimelineOpen toggles the flag", () => {
    const initial = stateWith(
      "chat-1",
      makeRuntime({ timeline_open: false }),
    );
    const state1 = reducer(
      initial,
      toggleTimelineOpen({ chatId: "chat-1" }),
    );
    expect(state1.runtimes["chat-1"]!.timeline_open).toBe(true);
    const state2 = reducer(
      state1,
      toggleTimelineOpen({ chatId: "chat-1" }),
    );
    expect(state2.runtimes["chat-1"]!.timeline_open).toBe(false);
  });

  test("setTimelineFilterSource sets the source filter", () => {
    const initial = stateWith("chat-1", makeRuntime());
    const state = reducer(
      initial,
      setTimelineFilterSource({ chatId: "chat-1", source: "agent" }),
    );
    expect(state.runtimes["chat-1"]!.timeline_filter_source).toBe("agent");
  });

  test("setTimelineFilterType sets the type filter", () => {
    const initial = stateWith("chat-1", makeRuntime());
    const state = reducer(
      initial,
      setTimelineFilterType({ chatId: "chat-1", type: "click" }),
    );
    expect(state.runtimes["chat-1"]!.timeline_filter_type).toBe("click");
  });

  test("setTimelineFilterType clears to null", () => {
    const initial = stateWith(
      "chat-1",
      makeRuntime({ timeline_filter_type: "click" }),
    );
    const state = reducer(
      initial,
      setTimelineFilterType({ chatId: "chat-1", type: null }),
    );
    expect(state.runtimes["chat-1"]!.timeline_filter_type).toBeNull();
  });

  test("setBrowserRuntime includes timeline fields", () => {
    const runtime = makeRuntime();
    const state = reducer(
      undefined,
      setBrowserRuntime({ chatId: "chat-1", runtime }),
    );
    const rt = state.runtimes["chat-1"]!;
    expect(rt.timeline).toEqual([]);
    expect(rt.timeline_open).toBe(false);
    expect(rt.timeline_filter_source).toBe("all");
    expect(rt.timeline_filter_type).toBeNull();
  });
});
