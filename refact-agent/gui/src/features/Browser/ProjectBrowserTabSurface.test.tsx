import { http, HttpResponse } from "msw";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { server } from "../../utils/mockServer";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
  waitFor,
} from "../../utils/test-utils";
import { browserApi } from "../../services/refact/browser";
import { applyChatEvent } from "../Chat/Thread/actions";
import { useProjectBrowserAutoOpen } from "../Workspace/useProjectBrowserAutoOpen";
import {
  openTab,
  selectBrowserViewOwners,
  showBrowserTabForChat,
} from "../Workspace/workspaceSlice";
import { framePosition } from "./projectBrowserFrame";
import { PaneSurfaceActivityProvider } from "../Workspace/PaneHeaderContext";
import { ProjectBrowserTabSurface } from "./ProjectBrowserTabSurface";
import { selectBrowserTabAwaitingStatus } from "./browserViews";
import { BrowserToggleButton } from "../../components/Buttons/BrowserToggleButton";

const tab = (tabId: string, url: string, title: string) => ({
  tab_id: tabId,
  url,
  title,
  active: false,
  name: null,
  opened_by_chat: null,
  last_chat: null,
  last_used_ms: null,
});

const frame = {
  runtime_id: "runtime-a",
  tab_id: "tab-b",
  mime: "image/png",
  data: "iVBORw0KGgo=",
  width: 600,
  height: 300,
  viewport: { width: 1200, height: 600 },
  url: "https://example.test/b",
  title: "Example B",
};

const liveStatus = {
  project_id: "test-project",
  running: true,
  connected: true,
  runtime_id: "runtime-a",
  active_tab: "tab-a",
  tabs: [
    tab("tab-a", "https://example.test/a", "Example A"),
    tab("tab-b", frame.url, frame.title),
  ],
};

let migrationRequests = 0;

beforeEach(() => {
  migrationRequests = 0;
  server.use(
    http.post("*/v1/browser/project/migration/*", () => {
      migrationRequests += 1;
      return HttpResponse.json({});
    }),
  );
  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get: () => 800,
  });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get: () => 400,
  });
});

afterEach(() => {
  Reflect.deleteProperty(HTMLElement.prototype, "clientWidth");
  Reflect.deleteProperty(HTMLElement.prototype, "clientHeight");
});

describe("Project browser tab surface", () => {
  it("maps displayed screenshot pixels to CSS viewport coordinates and rejects letterboxing", () => {
    const bounds = { left: 10, top: 20, width: 300, height: 200 };
    expect(framePosition(frame, bounds, 160, 120)).toEqual({ x: 600, y: 300 });
    expect(framePosition(frame, bounds, 160, 25)).toBeNull();
  });

  it("requests frames and resizes for its own tab, not the tool's active tab", async () => {
    const frames: unknown[] = [];
    const resizes: unknown[] = [];
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json(liveStatus),
      ),
      http.post("*/v1/browser/project/frame", async ({ request }) => {
        frames.push(await request.json());
        return HttpResponse.json(frame);
      }),
      http.post("*/v1/browser/project/resize", async ({ request }) => {
        resizes.push(await request.json());
        return HttpResponse.json({ applied: true });
      }),
    );
    render(<ProjectBrowserTabSurface tabId="tab-b" />);
    await screen.findByRole("img", { name: "Browser page" });
    expect(frames[0]).toEqual({ tab_id: "tab-b" });
    expect(resizes[0]).toEqual({ width: 800, height: 400, tab_id: "tab-b" });
    expect(screen.getByRole("textbox", { name: "Address" })).toHaveValue(
      frame.url,
    );
  });

  it("sends ordered key and paste input fenced on the runtime and tab", async () => {
    const inputs: unknown[] = [];
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json(liveStatus),
      ),
      http.post("*/v1/browser/project/frame", () => HttpResponse.json(frame)),
      http.post("*/v1/browser/project/resize", () =>
        HttpResponse.json({ applied: true }),
      ),
      http.post("*/v1/browser/project/input", async ({ request }) => {
        inputs.push(await request.json());
        return HttpResponse.json({ ok: true });
      }),
    );
    render(<ProjectBrowserTabSurface tabId="tab-b" />);
    const image = await screen.findByRole("img", { name: "Browser page" });
    fireEvent.keyDown(image, { key: "Backspace" });
    fireEvent.paste(image, { clipboardData: { getData: () => "hello" } });
    await waitFor(() =>
      expect(inputs).toEqual([
        {
          runtime_id: "runtime-a",
          tab_id: "tab-b",
          kind: "key",
          key: "Backspace",
          modifiers: [],
        },
        {
          runtime_id: "runtime-a",
          tab_id: "tab-b",
          kind: "text",
          text: "hello",
        },
      ]),
    );
  });

  it("navigates its own tab", async () => {
    const navigations: unknown[] = [];
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json(liveStatus),
      ),
      http.post("*/v1/browser/project/frame", () => HttpResponse.json(frame)),
      http.post("*/v1/browser/project/resize", () =>
        HttpResponse.json({ applied: true }),
      ),
      http.post("*/v1/browser/project/navigate", async ({ request }) => {
        navigations.push(await request.json());
        return HttpResponse.json(liveStatus);
      }),
    );
    render(<ProjectBrowserTabSurface tabId="tab-b" />);
    fireEvent.click(await screen.findByRole("button", { name: "Reload" }));
    await waitFor(() =>
      expect(navigations).toEqual([{ op: "reload", tab_id: "tab-b" }]),
    );
  });

  it("shows a closed state when the Chromium tab is gone, without any migration UI", async () => {
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json({ ...liveStatus, tabs: [liveStatus.tabs[0]] }),
      ),
    );
    render(<ProjectBrowserTabSurface tabId="tab-b" />);
    expect(
      await screen.findByRole("heading", {
        name: "This browser tab is closed",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText(/Browser rail/)).toBeInTheDocument();
    expect(screen.queryByText(/profile/i)).not.toBeInTheDocument();
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
    expect(migrationRequests).toBe(0);
  });

  it("shows a closed state when the engine reports the tab as gone", async () => {
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json(liveStatus),
      ),
      http.post("*/v1/browser/project/resize", () =>
        HttpResponse.json({ applied: true }),
      ),
      http.post("*/v1/browser/project/frame", () =>
        HttpResponse.json({ detail: "unknown tab" }, { status: 410 }),
      ),
    );
    render(<ProjectBrowserTabSurface tabId="tab-b" />);
    expect(
      await screen.findByRole("heading", {
        name: "This browser tab is closed",
      }),
    ).toBeInTheDocument();
  });

  it("shows a closed tab and never starts the browser when it is not running", async () => {
    let starts = 0;
    let statuses = 0;
    server.use(
      http.post("*/v1/browser/project/status", () => {
        statuses += 1;
        return HttpResponse.json({
          project_id: "test-project",
          running: false,
          connected: false,
          tabs: [],
        });
      }),
      http.post("*/v1/browser/project/start", () => {
        starts += 1;
        return HttpResponse.json({ ...liveStatus, launched: true });
      }),
    );
    render(<ProjectBrowserTabSurface tabId="tab-b" />);
    expect(
      await screen.findByRole("heading", {
        name: "This browser tab is closed",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText(/Browser rail/)).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Start browser" }),
    ).not.toBeInTheDocument();
    await waitFor(() => expect(statuses).toBeGreaterThanOrEqual(1));
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(starts).toBe(0);
    expect(migrationRequests).toBe(0);
  });

  it("keeps the last frame and retries when the engine answers 409", async () => {
    let frames = 0;
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json(liveStatus),
      ),
      http.post("*/v1/browser/project/resize", () =>
        HttpResponse.json({ applied: true }),
      ),
      http.post("*/v1/browser/project/frame", () => {
        frames += 1;
        return frames === 1
          ? HttpResponse.json(frame)
          : HttpResponse.json(
              {
                detail:
                  "the project browser is busy with another operation; try again",
              },
              { status: 409 },
            );
      }),
    );
    render(<ProjectBrowserTabSurface tabId="tab-b" />);
    await screen.findByRole("img", { name: "Browser page" });
    await waitFor(() => expect(frames).toBeGreaterThanOrEqual(3));
    expect(screen.getByRole("img", { name: "Browser page" })).toBeVisible();
    expect(
      screen.queryByRole("heading", { name: "This browser tab is closed" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("does not treat a busy status without tabs as a closed tab", async () => {
    let statuses = 0;
    server.use(
      http.post("*/v1/browser/project/status", () => {
        statuses += 1;
        return HttpResponse.json({ ...liveStatus, busy: true, tabs: [] });
      }),
    );
    render(<ProjectBrowserTabSurface tabId="tab-b" />);
    expect(
      await screen.findByText("Waiting for a browser frame…"),
    ).toBeInTheDocument();
    await waitFor(() => expect(statuses).toBeGreaterThanOrEqual(1));
    expect(
      screen.queryByRole("heading", { name: "This browser tab is closed" }),
    ).not.toBeInTheDocument();
  });

  it("does not poll frames while its pane surface is inactive", async () => {
    const requested: string[] = [];
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json(liveStatus),
      ),
      http.post("*/v1/browser/project/resize", () =>
        HttpResponse.json({ applied: true }),
      ),
      http.post("*/v1/browser/project/frame", async ({ request }) => {
        const body = (await request.json()) as { tab_id: string };
        requested.push(body.tab_id);
        return HttpResponse.json({ ...frame, tab_id: body.tab_id });
      }),
    );
    render(
      <>
        <PaneSurfaceActivityProvider active>
          <ProjectBrowserTabSurface tabId="tab-a" />
        </PaneSurfaceActivityProvider>
        <PaneSurfaceActivityProvider active={false}>
          <ProjectBrowserTabSurface tabId="tab-b" />
        </PaneSurfaceActivityProvider>
      </>,
    );
    await waitFor(
      () =>
        expect(
          requested.filter((id) => id === "tab-a").length,
        ).toBeGreaterThanOrEqual(3),
      { timeout: 3000 },
    );
    expect(requested).not.toContain("tab-b");
  });
});

function HintSubscriber() {
  useProjectBrowserAutoOpen();
  return null;
}

const stopped = {
  project_id: "test-project",
  running: false,
  connected: false,
  tabs: [],
};

describe("Project browser tab surface after a restart or a hint", () => {
  it("does not show a hinted tab as closed while a fresher status is pending", async () => {
    let body: Record<string, unknown> = stopped;
    let release: () => void = () => undefined;
    let gate: Promise<void> = Promise.resolve();
    server.use(
      http.post("*/v1/browser/project/status", async () => {
        await gate;
        return HttpResponse.json(body);
      }),
      http.post("*/v1/browser/project/resize", () =>
        HttpResponse.json({ applied: true }),
      ),
      http.post("*/v1/browser/project/frame", () => HttpResponse.json(frame)),
    );
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    const { store } = render(<HintSubscriber />, {
      preloadedState: { chat },
    });
    store.dispatch(openTab(`chat:${chatId}`));
    await store.dispatch(
      browserApi.endpoints.projectBrowserStatus.initiate(undefined),
    );
    body = liveStatus;
    gate = new Promise((resolve) => {
      release = resolve;
    });
    store.dispatch(
      applyChatEvent({
        chat_id: chatId,
        seq: "1",
        type: "message_added",
        index: 0,
        message: {
          role: "tool",
          tool_call_id: "call-1",
          content: "done",
          browser_project: {
            open_browser_tab: true,
            runtime_id: "runtime-a",
            tab_id: "tab-b",
            tab_name: null,
          },
        } as never,
      }),
    );
    expect(selectBrowserViewOwners(store.getState())).toEqual({
      "tab-b": chatId,
    });
    render(<ProjectBrowserTabSurface tabId="tab-b" />, { store });
    await new Promise((resolve) => setTimeout(resolve, 150));
    expect(
      screen.queryByRole("heading", { name: "This browser tab is closed" }),
    ).not.toBeInTheDocument();
    release();
    expect(
      await screen.findByRole("img", { name: "Browser page" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "This browser tab is closed" }),
    ).not.toBeInTheDocument();
  });

  it("refetches the shared status when a view mounts", async () => {
    let statuses = 0;
    server.use(
      http.post("*/v1/browser/project/status", () => {
        statuses += 1;
        return HttpResponse.json(liveStatus);
      }),
      http.post("*/v1/browser/project/resize", () =>
        HttpResponse.json({ applied: true }),
      ),
      http.post("*/v1/browser/project/frame", () => HttpResponse.json(frame)),
    );
    const chat = createDefaultChatState();
    const { store } = render(<HintSubscriber />, {
      preloadedState: { chat },
    });
    await store.dispatch(
      browserApi.endpoints.projectBrowserStatus.initiate(undefined),
    );
    const before = statuses;
    render(<ProjectBrowserTabSurface tabId="tab-b" />, { store });
    await waitFor(() => expect(statuses).toBeGreaterThan(before));
  });

  it("opens a live browser tab for the owning chat from the closed state", async () => {
    let running = false;
    let starts = 0;
    const tabRequests: unknown[] = [];
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    const runningStatus = () => ({
      ...liveStatus,
      tabs: [{ ...liveStatus.tabs[0], tab_id: "tab-new", last_chat: chatId }],
    });
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json(running ? runningStatus() : stopped),
      ),
      http.post("*/v1/browser/project/start", () => {
        starts += 1;
        running = true;
        return HttpResponse.json({ ...liveStatus, tabs: [] });
      }),
      http.post("*/v1/browser/project/tab", async ({ request }) => {
        tabRequests.push(await request.json());
        return HttpResponse.json({
          ...runningStatus(),
          new_tab_id: "tab-new",
        });
      }),
    );
    const { store } = render(<ProjectBrowserTabSurface tabId="tab-dead" />, {
      preloadedState: { chat },
    });
    store.dispatch(openTab(`chat:${chatId}`));
    store.dispatch(showBrowserTabForChat({ chatId, tabId: "tab-dead" }));
    await screen.findByRole("heading", { name: "This browser tab is closed" });
    fireEvent.click(
      screen.getByRole("button", { name: "Open browser for this chat" }),
    );
    await waitFor(() =>
      expect(selectBrowserViewOwners(store.getState())).toEqual({
        "tab-new": chatId,
      }),
    );
    expect(starts).toBe(1);
    expect(tabRequests).toEqual([{ op: "new", chat_id: chatId }]);
  });
});

const hintFor = (chatId: string, tabId: string) =>
  applyChatEvent({
    chat_id: chatId,
    seq: "1",
    type: "message_added",
    index: 0,
    message: {
      role: "tool",
      tool_call_id: "call-1",
      content: "done",
      browser_project: {
        open_browser_tab: true,
        runtime_id: "runtime-a",
        tab_id: tabId,
        tab_name: null,
      },
    } as never,
  });

describe("Project browser tab surface: review round 4", () => {
  it.each([
    [
      "start",
      stopped,
      { start: { status: 500, detail: "Chrome is not installed" } },
      "Chrome is not installed",
    ],
    [
      "tab",
      stopped,
      { tab: { status: 409, detail: "Too many browser tabs" } },
      "Too many browser tabs",
    ],
  ] as const)(
    "keeps the closed pane and shows the %s failure when reopening",
    async (_label, statusBody, failures, detail) => {
      server.use(
        http.post("*/v1/browser/project/status", () =>
          HttpResponse.json(statusBody),
        ),
        http.post("*/v1/browser/project/start", () =>
          "start" in failures
            ? HttpResponse.json(
                { detail: failures.start.detail },
                { status: failures.start.status },
              )
            : HttpResponse.json({ ...liveStatus, tabs: [] }),
        ),
        http.post("*/v1/browser/project/tab", () =>
          "tab" in failures
            ? HttpResponse.json(
                { detail: failures.tab.detail },
                { status: failures.tab.status },
              )
            : HttpResponse.json(liveStatus),
        ),
      );
      const chat = createDefaultChatState();
      const chatId = chat.current_thread_id;
      const { store } = render(<ProjectBrowserTabSurface tabId="tab-dead" />, {
        preloadedState: { chat },
      });
      store.dispatch(openTab(`chat:${chatId}`));
      store.dispatch(showBrowserTabForChat({ chatId, tabId: "tab-dead" }));
      await screen.findByRole("heading", {
        name: "This browser tab is closed",
      });
      fireEvent.click(
        screen.getByRole("button", { name: "Open browser for this chat" }),
      );
      expect(await screen.findByRole("alert")).toHaveTextContent(detail);
      expect(selectBrowserViewOwners(store.getState())).toEqual({
        "tab-dead": chatId,
      });
      expect(
        screen.getByRole("heading", { name: "This browser tab is closed" }),
      ).toBeInTheDocument();
    },
  );

  it("shows the closed state when the browser is still not running after a hint", async () => {
    let statuses = 0;
    server.use(
      http.post("*/v1/browser/project/status", () => {
        statuses += 1;
        return HttpResponse.json(stopped);
      }),
    );
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    const { store } = render(<HintSubscriber />, {
      preloadedState: { chat },
    });
    store.dispatch(openTab(`chat:${chatId}`));
    store.dispatch(hintFor(chatId, "tab-b"));
    render(<ProjectBrowserTabSurface tabId="tab-b" />, { store });
    expect(
      await screen.findByRole("heading", {
        name: "This browser tab is closed",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Open browser for this chat" }),
    ).toBeInTheDocument();
    expect(statuses).toBeGreaterThanOrEqual(1);
  });

  it("ignores the status of a mutation that was already running when the hint arrived", async () => {
    let releaseNavigate: () => void = () => undefined;
    const navigateGate = new Promise<void>((resolve) => {
      releaseNavigate = resolve;
    });
    let releaseStatus: () => void = () => undefined;
    let statusGate: Promise<void> = Promise.resolve();
    const before = { ...liveStatus, tabs: [liveStatus.tabs[0]] };
    let body: Record<string, unknown> = before;
    server.use(
      http.post("*/v1/browser/project/status", async () => {
        await statusGate;
        return HttpResponse.json(body);
      }),
      http.post("*/v1/browser/project/navigate", async () => {
        await navigateGate;
        return HttpResponse.json(before);
      }),
      http.post("*/v1/browser/project/resize", () =>
        HttpResponse.json({ applied: true }),
      ),
      http.post("*/v1/browser/project/frame", () => HttpResponse.json(frame)),
    );
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    const { store } = render(<HintSubscriber />, {
      preloadedState: { chat },
    });
    store.dispatch(openTab(`chat:${chatId}`));
    await store.dispatch(
      browserApi.endpoints.projectBrowserStatus.initiate(undefined),
    );
    const navigation = store.dispatch(
      browserApi.endpoints.projectBrowserNavigate.initiate({
        op: "reload",
        tab_id: "tab-a",
      }),
    );
    statusGate = new Promise((resolve) => {
      releaseStatus = resolve;
    });
    body = liveStatus;
    store.dispatch(hintFor(chatId, "tab-b"));
    render(<ProjectBrowserTabSurface tabId="tab-b" />, { store });
    releaseNavigate();
    await navigation;
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(
      screen.queryByRole("heading", { name: "This browser tab is closed" }),
    ).not.toBeInTheDocument();
    releaseStatus();
    expect(
      await screen.findByRole("img", { name: "Browser page" }),
    ).toBeInTheDocument();
  });
});

describe("Project browser tab surface: review round 5", () => {
  it("keeps a dead view in a running browser and shows the failure of reopening it", async () => {
    let statuses = 0;
    let releaseTab: () => void = () => undefined;
    const tabGate = new Promise<void>((resolve) => {
      releaseTab = resolve;
    });
    server.use(
      http.post("*/v1/browser/project/status", () => {
        statuses += 1;
        return HttpResponse.json({ ...liveStatus, tabs: [] });
      }),
      http.post("*/v1/browser/project/tab", async () => {
        await tabGate;
        return HttpResponse.json(
          { detail: "Too many browser tabs" },
          { status: 409 },
        );
      }),
    );
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    const { store } = render(<ProjectBrowserTabSurface tabId="tab-dead" />, {
      preloadedState: { chat },
    });
    store.dispatch(openTab(`chat:${chatId}`));
    store.dispatch(showBrowserTabForChat({ chatId, tabId: "tab-dead" }));
    await screen.findByRole("heading", { name: "This browser tab is closed" });
    await waitFor(() => expect(statuses).toBeGreaterThanOrEqual(1));
    expect(selectBrowserViewOwners(store.getState())).toEqual({
      "tab-dead": chatId,
    });
    const before = statuses;
    fireEvent.click(
      screen.getByRole("button", { name: "Open browser for this chat" }),
    );
    await waitFor(() => expect(statuses).toBeGreaterThan(before));
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(selectBrowserViewOwners(store.getState())).toEqual({
      "tab-dead": chatId,
    });
    releaseTab();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Too many browser tabs",
    );
    expect(selectBrowserViewOwners(store.getState())).toEqual({
      "tab-dead": chatId,
    });
  });

  it("confirms a hint for an unmounted view after an in-flight status request settles", async () => {
    let calls = 0;
    let release: () => void = () => undefined;
    let gate: Promise<void> = Promise.resolve();
    server.use(
      http.post("*/v1/browser/project/status", async () => {
        calls += 1;
        await gate;
        return HttpResponse.json(liveStatus);
      }),
    );
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    const { store } = render(<HintSubscriber />, {
      preloadedState: { chat },
    });
    store.dispatch(openTab(`chat:${chatId}`));
    gate = new Promise((resolve) => {
      release = resolve;
    });
    const inFlight = store.dispatch(
      browserApi.endpoints.projectBrowserStatus.initiate(undefined),
    );
    await waitFor(() => expect(calls).toBe(1));
    store.dispatch(hintFor(chatId, "tab-b"));
    expect(selectBrowserTabAwaitingStatus(store.getState(), "tab-b")).toBe(
      true,
    );
    gate = Promise.resolve();
    release();
    await inFlight;
    await waitFor(() => expect(calls).toBe(2));
    await waitFor(() =>
      expect(selectBrowserTabAwaitingStatus(store.getState(), "tab-b")).toBe(
        false,
      ),
    );
  });
});

describe("Project browser tab surface: review round 6", () => {
  it("runs one show at a time for a chat across the closed view and the composer toggle", async () => {
    let starts = 0;
    const tabRequests: unknown[] = [];
    let releaseStart: () => void = () => undefined;
    const startGate = new Promise<void>((resolve) => {
      releaseStart = resolve;
    });
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json(stopped),
      ),
      http.post("*/v1/browser/project/start", async () => {
        starts += 1;
        await startGate;
        return HttpResponse.json({ ...liveStatus, tabs: [] });
      }),
      http.post("*/v1/browser/project/tab", async ({ request }) => {
        tabRequests.push(await request.json());
        return HttpResponse.json({
          ...liveStatus,
          tabs: [
            { ...liveStatus.tabs[0], tab_id: "tab-new", last_chat: chatId },
          ],
          new_tab_id: "tab-new",
        });
      }),
    );
    const { store } = render(
      <>
        <ProjectBrowserTabSurface tabId="tab-dead" />
        <BrowserToggleButton chatId={chatId} />
      </>,
      { preloadedState: { chat } },
    );
    store.dispatch(openTab(`chat:${chatId}`));
    store.dispatch(showBrowserTabForChat({ chatId, tabId: "tab-dead" }));
    await screen.findByRole("heading", { name: "This browser tab is closed" });
    fireEvent.click(
      screen.getByRole("button", { name: "Open browser for this chat" }),
    );
    await waitFor(() => expect(starts).toBe(1));
    expect(screen.getByRole("button", { name: "Show browser" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Show browser" }));
    fireEvent.click(
      screen.getByRole("button", { name: "Open browser for this chat" }),
    );
    await new Promise((resolve) => setTimeout(resolve, 50));
    releaseStart();
    await waitFor(() =>
      expect(selectBrowserViewOwners(store.getState())).toEqual({
        "tab-new": chatId,
      }),
    );
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(starts).toBe(1);
    expect(tabRequests).toEqual([{ op: "new", chat_id: chatId }]);
  });
});
