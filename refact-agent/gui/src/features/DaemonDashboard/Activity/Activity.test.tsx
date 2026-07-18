import { act } from "react-dom/test-utils";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../../../app/store";
import type {
  DaemonEvent,
  DaemonWorker,
} from "../../../services/refact/daemon";
import { server } from "../../../utils/mockServer";
import { render, screen } from "../../../utils/test-utils";
import { daemonEventsReceived, navigateDashboard } from "../dashboardSlice";
import { ActivityPage } from "./ActivityPage";
import {
  appendLogLine,
  filterDaemonEvents,
  mergeLogLines,
  timelineFollowAfterScroll,
} from "./activityState";

function daemonEvent(
  seq: number,
  kind: string,
  projectId: string | null,
  payload: unknown = undefined,
): DaemonEvent {
  return {
    seq,
    ts_ms: seq,
    kind,
    project_id: projectId,
    payload: payload ?? { seq },
  };
}

describe("Activity timeline state", () => {
  const events = [
    daemonEvent(1, "worker_started", "p1"),
    daemonEvent(2, "worker_stopped", "p2"),
    daemonEvent(3, "worker_started", "p2"),
  ];

  it("filters the event ring by selected kinds and project", () => {
    expect(
      filterDaemonEvents(events, new Set(["worker_started"]), "p2"),
    ).toEqual([events[2]]);
    expect(filterDaemonEvents(events, new Set(), null)).toEqual(events);
  });

  it("keeps follow pinned at the top and disables it after manual scroll", () => {
    expect(timelineFollowAfterScroll(true, 0)).toBe(true);
    expect(timelineFollowAfterScroll(true, 12)).toBe(false);
    expect(timelineFollowAfterScroll(false, 0)).toBe(false);
  });
});

describe("Activity log tail state", () => {
  it("caps retained lines at the requested tail size", () => {
    const result = ["one", "two", "three", "four"].reduce<string[]>(
      (lines, line) => appendLogLine(lines, line, false, 3),
      [],
    );
    expect(result).toEqual(["two", "three", "four"]);
  });

  it("does not append streamed lines while paused", () => {
    const lines = ["existing"];
    expect(appendLogLine(lines, "ignored", true)).toBe(lines);
  });

  it("flushes buffered paused lines through the cap on resume", () => {
    expect(mergeLogLines(["one", "two"], ["three", "four"], 3)).toEqual([
      "two",
      "three",
      "four",
    ]);
  });
});

class RecordingEventSource {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;
  static urls: string[] = [];
  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSED = 2;
  onerror: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onopen: ((event: Event) => void) | null = null;
  readyState = RecordingEventSource.CONNECTING;
  url: string;

  constructor(url: string | URL) {
    this.url = String(url);
    RecordingEventSource.urls.push(this.url);
  }

  addEventListener() {
    return undefined;
  }

  removeEventListener() {
    return undefined;
  }

  close() {
    this.readyState = RecordingEventSource.CLOSED;
  }

  dispatchEvent() {
    return true;
  }
}

const dashboardConfig = {
  apiKey: "",
  host: "web" as const,
  lspPort: 8488,
  lspUrl: "http://127.0.0.1:8488",
  surface: "dashboard" as const,
  themeProps: {},
};

function daemonWorker(projectId: string, slug: string): DaemonWorker {
  return {
    project_id: projectId,
    slug,
    root: `/tmp/${slug}`,
    pinned: false,
    last_active_ms: null,
    state: "running",
    pid: null,
    http_port: null,
    lsp_port: null,
    lsp_clients: 0,
    busy_chats: 0,
    exec_running: 0,
    live_proxy_streams: 0,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: null,
    last_error: null,
  };
}

describe("ActivityPage navigation params", () => {
  beforeEach(() => {
    RecordingEventSource.urls = [];
    vi.stubGlobal("EventSource", RecordingEventSource);
    server.use(
      http.get("*/daemon/v1/workers", () =>
        HttpResponse.json([
          daemonWorker("p1", "alpha"),
          daemonWorker("p2", "beta"),
        ]),
      ),
    );
  });

  function setUpActivityStore() {
    const store = setUpStore({ config: dashboardConfig });
    store.dispatch(
      daemonEventsReceived([
        daemonEvent(1, "worker_started", "p1", { message: "alpha event" }),
        daemonEvent(2, "worker_started", "p2", { message: "beta event" }),
      ]),
    );
    return store;
  }

  it("preselects the timeline filter and log pane from the projectId param", async () => {
    const store = setUpActivityStore();
    store.dispatch(
      navigateDashboard({ page: "activity", params: { projectId: "p2" } }),
    );

    render(<ActivityPage />, { store });

    expect(await screen.findByText("beta event")).toBeInTheDocument();
    expect(screen.queryByText("alpha event")).toBeNull();
    expect(RecordingEventSource.urls.at(-1)).toContain("project_id=p2");

    act(() => {
      store.dispatch(
        navigateDashboard({ page: "activity", params: { projectId: "p1" } }),
      );
    });

    expect(await screen.findByText("alpha event")).toBeInTheDocument();
    expect(screen.queryByText("beta event")).toBeNull();
    expect(RecordingEventSource.urls.at(-1)).toContain("project_id=p1");
  });

  it("keeps the unfiltered view when no projectId param is present", async () => {
    const store = setUpActivityStore();
    store.dispatch(navigateDashboard({ page: "activity", params: {} }));

    render(<ActivityPage />, { store });

    expect(await screen.findByText("alpha event")).toBeInTheDocument();
    expect(screen.getByText("beta event")).toBeInTheDocument();
    expect(RecordingEventSource.urls.at(-1)).not.toContain("project_id");
  });
});
