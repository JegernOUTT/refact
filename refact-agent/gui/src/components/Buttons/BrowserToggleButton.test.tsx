import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { browserApi } from "../../services/refact/browser";
import { browserHintsHandled } from "../../features/Browser/browserViews";
import { server } from "../../utils/mockServer";
import { act } from "react-dom/test-utils";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
  waitFor,
} from "../../utils/test-utils";
import {
  openTab,
  selectBrowserViewOwners,
  showBrowserTabForChat,
} from "../../features/Workspace/workspaceSlice";
import { BrowserDockPanel } from "../../features/Browser/BrowserDockPanel";
import { STATUS_REFRESH_FAILED } from "../../features/Browser/useProjectBrowserTabActions";
import { PROJECT_BROWSER_STATUS_POLL_MS } from "../../features/Browser/useProjectBrowserStatus";
import { BrowserToggleButton } from "./BrowserToggleButton";

const tab = (tabId: string, fields: Record<string, unknown>) => ({
  tab_id: tabId,
  url: "about:blank",
  title: "",
  active: false,
  name: null,
  opened_by_chat: null,
  last_chat: null,
  last_used_ms: null,
  ...fields,
});

function setup(tabs: (chatId: string) => ReturnType<typeof tab>[]) {
  const chat = createDefaultChatState();
  const chatId = chat.current_thread_id;
  const requests: unknown[] = [];
  const status = () => ({
    project_id: "test-project",
    running: true,
    connected: true,
    runtime_id: "runtime-a",
    active_tab: null,
    tabs: tabs(chatId),
  });
  server.use(
    http.post("*/v1/browser/project/status", () => HttpResponse.json(status())),
    http.post("*/v1/browser/project/tab", async ({ request }) => {
      requests.push(await request.json());
      return HttpResponse.json({
        ...status(),
        tabs: [...status().tabs, tab("tab-new", {})],
        new_tab_id: "tab-new",
      });
    }),
  );
  const view = render(<BrowserToggleButton chatId={chatId} />, {
    preloadedState: { chat },
  });
  view.store.dispatch(openTab(`chat:${chatId}`));
  return { ...view, chatId, requests };
}

describe("Composer browser toggle", () => {
  it("hides the chat's browser views when they are shown", async () => {
    const { store, chatId } = setup(() => [tab("tab-a", {})]);
    act(() => {
      store.dispatch(showBrowserTabForChat({ chatId, tabId: "tab-a" }));
    });
    fireEvent.click(
      await screen.findByRole("button", { name: "Hide browser" }),
    );
    expect(selectBrowserViewOwners(store.getState())).toEqual({});
    expect(
      await screen.findByRole("button", { name: "Show browser" }),
    ).toBeInTheDocument();
  });

  it("reopens the tab the chat used last, like the engine's own-tab rule", async () => {
    const { store, chatId, requests } = setup((chatId) => [
      tab("tab-used", { last_chat: chatId, last_used_ms: 10 }),
      tab("tab-opened", { opened_by_chat: chatId, last_used_ms: 20 }),
      tab("tab-other", { last_chat: "someone-else", last_used_ms: 30 }),
    ]);
    fireEvent.click(screen.getByRole("button", { name: "Show browser" }));
    await waitFor(() =>
      expect(selectBrowserViewOwners(store.getState())).toEqual({
        "tab-used": chatId,
      }),
    );
    expect(requests).toEqual([]);
  });

  it("creates a new tab when the only tab it opened was last used by another chat", async () => {
    const { store, chatId, requests } = setup((chatId) => [
      tab("tab-taken", {
        opened_by_chat: chatId,
        last_chat: "someone-else",
        last_used_ms: 30,
      }),
    ]);
    fireEvent.click(screen.getByRole("button", { name: "Show browser" }));
    await waitFor(() =>
      expect(selectBrowserViewOwners(store.getState())).toEqual({
        "tab-new": chatId,
      }),
    );
    expect(requests).toEqual([{ op: "new", chat_id: chatId }]);
  });

  it("reopens the last own tab while the browser reports busy", async () => {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    const requests: unknown[] = [];
    let busy = false;
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json({
          project_id: "test-project",
          running: true,
          connected: true,
          busy,
          runtime_id: "runtime-a",
          active_tab: null,
          tabs: busy
            ? []
            : [tab("tab-own", { last_chat: chatId, last_used_ms: 5 })],
        }),
      ),
      http.post("*/v1/browser/project/tab", async ({ request }) => {
        requests.push(await request.json());
        return HttpResponse.json(
          { detail: "the project browser is busy" },
          { status: 409 },
        );
      }),
    );
    const { store } = render(<BrowserToggleButton chatId={chatId} />, {
      preloadedState: { chat },
    });
    store.dispatch(openTab(`chat:${chatId}`));
    await act(async () => {
      await store.dispatch(
        browserApi.endpoints.projectBrowserStatus.initiate(undefined),
      );
    });
    busy = true;
    fireEvent.click(screen.getByRole("button", { name: "Show browser" }));
    await waitFor(() =>
      expect(selectBrowserViewOwners(store.getState())).toEqual({
        "tab-own": chatId,
      }),
    );
    expect(requests).toEqual([]);
  });

  it("announces a failed action to assistive technology", async () => {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json({
          project_id: "test-project",
          running: true,
          connected: true,
          runtime_id: "runtime-a",
          active_tab: null,
          tabs: [],
        }),
      ),
      http.post("*/v1/browser/project/tab", () =>
        HttpResponse.json({ detail: "Tab limit reached" }, { status: 409 }),
      ),
    );
    render(<BrowserToggleButton chatId={chatId} />, {
      preloadedState: { chat },
    });
    fireEvent.click(screen.getByRole("button", { name: "Show browser" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Tab limit reached",
    );
    expect(
      screen.getByRole("button", { name: "Show browser" }),
    ).toHaveAccessibleDescription("Tab limit reached");
  });

  it("creates a new tab for a chat without tabs and opens it", async () => {
    const { store, chatId, requests } = setup(() => [
      tab("tab-other", { last_chat: "someone-else", last_used_ms: 30 }),
    ]);
    fireEvent.click(screen.getByRole("button", { name: "Show browser" }));
    await waitFor(() =>
      expect(selectBrowserViewOwners(store.getState())).toEqual({
        "tab-new": chatId,
      }),
    );
    expect(requests).toEqual([{ op: "new", chat_id: chatId }]);
  });

  it("offers to show the browser when the chat only has dead views and replaces them in one click", async () => {
    const { store, chatId, requests } = setup((chatId) => [
      tab("tab-own", { last_chat: chatId, last_used_ms: 5 }),
    ]);
    act(() => {
      store.dispatch(showBrowserTabForChat({ chatId, tabId: "tab-dead" }));
    });
    const button = await screen.findByRole("button", { name: "Show browser" });
    fireEvent.click(button);
    await waitFor(() =>
      expect(selectBrowserViewOwners(store.getState())).toEqual({
        "tab-own": chatId,
      }),
    );
    expect(requests).toEqual([]);
    expect(
      await screen.findByRole("button", { name: "Hide browser" }),
    ).toBeInTheDocument();
  });

  it("starts the browser and creates a tab when the chat's views died with the browser", async () => {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    let running = false;
    let starts = 0;
    const requests: unknown[] = [];
    const live = (tabs: ReturnType<typeof tab>[]) => ({
      project_id: "test-project",
      running: true,
      connected: true,
      runtime_id: "runtime-b",
      active_tab: null,
      tabs,
    });
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json(
          running
            ? live([tab("tab-new", { last_chat: chatId })])
            : {
                project_id: "test-project",
                running: false,
                connected: false,
                tabs: [],
              },
        ),
      ),
      http.post("*/v1/browser/project/start", () => {
        starts += 1;
        running = true;
        return HttpResponse.json(live([]));
      }),
      http.post("*/v1/browser/project/tab", async ({ request }) => {
        requests.push(await request.json());
        return HttpResponse.json({
          ...live([tab("tab-new", { last_chat: chatId })]),
          new_tab_id: "tab-new",
        });
      }),
    );
    const { store } = render(<BrowserToggleButton chatId={chatId} />, {
      preloadedState: { chat },
    });
    act(() => {
      store.dispatch(openTab(`chat:${chatId}`));
      store.dispatch(showBrowserTabForChat({ chatId, tabId: "tab-dead" }));
    });
    fireEvent.click(
      await screen.findByRole("button", { name: "Show browser" }),
    );
    await waitFor(() =>
      expect(selectBrowserViewOwners(store.getState())).toEqual({
        "tab-new": chatId,
      }),
    );
    expect(starts).toBe(1);
    expect(requests).toEqual([{ op: "new", chat_id: chatId }]);
  });

  it("treats a just-hinted view as shown before its status confirms it", async () => {
    const { store, chatId } = setup(() => []);
    await act(async () => {
      await store.dispatch(
        browserApi.endpoints.projectBrowserStatus.initiate(undefined),
      );
    });
    server.use(
      http.post(
        "*/v1/browser/project/status",
        () => new Promise<never>(() => undefined),
      ),
    );
    await screen.findByRole("button", { name: "Show browser" });
    act(() => {
      store.dispatch(showBrowserTabForChat({ chatId, tabId: "tab-hinted" }));
      store.dispatch(
        browserHintsHandled({
          chatId,
          toolCallIds: ["call-1"],
          hintedTabId: "tab-hinted",
        }),
      );
    });
    expect(
      await screen.findByRole("button", { name: "Hide browser" }),
    ).toBeInTheDocument();
  });

  it("classifies dead views only from a status fetched after the click", async () => {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    const requests: unknown[] = [];
    let calls = 0;
    let release: () => void = () => undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const status = (tabs: ReturnType<typeof tab>[]) => ({
      project_id: "test-project",
      running: true,
      connected: true,
      runtime_id: "runtime-a",
      active_tab: null,
      tabs,
    });
    server.use(
      http.post("*/v1/browser/project/status", async () => {
        calls += 1;
        const call = calls;
        if (call === 2) await gate;
        return HttpResponse.json(
          status(
            call >= 3
              ? [tab("tab-x", { last_chat: chatId, last_used_ms: 1 })]
              : [],
          ),
        );
      }),
      http.post("*/v1/browser/project/tab", async ({ request }) => {
        requests.push(await request.json());
        return HttpResponse.json({
          ...status([]),
          new_tab_id: "tab-second",
        });
      }),
    );
    const { store } = render(<BrowserToggleButton chatId={chatId} />, {
      preloadedState: { chat },
    });
    act(() => {
      store.dispatch(openTab(`chat:${chatId}`));
      store.dispatch(showBrowserTabForChat({ chatId, tabId: "tab-x" }));
    });
    const button = await screen.findByRole("button", { name: "Show browser" });
    await waitFor(() => expect(calls).toBe(1));
    void store.dispatch(
      browserApi.endpoints.projectBrowserStatus.initiate(undefined, {
        forceRefetch: true,
        subscribe: false,
      }),
    );
    await waitFor(() => expect(calls).toBe(2));
    fireEvent.click(button);
    release();
    await waitFor(() => expect(calls).toBeGreaterThanOrEqual(3));
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Hide browser" }),
      ).toBeInTheDocument(),
    );
    expect(selectBrowserViewOwners(store.getState())).toEqual({
      "tab-x": chatId,
    });
    expect(requests).toEqual([]);
  });

  it("waits for an in-flight status poll instead of failing the click", async () => {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    let calls = 0;
    let release: () => void = () => undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    server.use(
      http.post("*/v1/browser/project/status", async () => {
        calls += 1;
        if (calls === 2) await gate;
        return HttpResponse.json({
          project_id: "test-project",
          running: true,
          connected: true,
          runtime_id: "runtime-a",
          active_tab: null,
          tabs: [tab("tab-own", { last_chat: chatId, last_used_ms: 1 })],
        });
      }),
    );
    const { store } = render(
      <>
        <BrowserDockPanel />
        <BrowserToggleButton chatId={chatId} />
      </>,
      { preloadedState: { chat } },
    );
    act(() => {
      store.dispatch(openTab(`chat:${chatId}`));
    });
    await waitFor(() => expect(calls).toBe(2), {
      timeout: PROJECT_BROWSER_STATUS_POLL_MS + 2000,
    });
    fireEvent.click(screen.getByRole("button", { name: "Show browser" }));
    await new Promise((resolve) => setTimeout(resolve, 50));
    release();
    await waitFor(() =>
      expect(selectBrowserViewOwners(store.getState())).toEqual({
        "tab-own": chatId,
      }),
    );
    expect(calls).toBeGreaterThanOrEqual(3);
    expect(screen.queryByText(STATUS_REFRESH_FAILED)).not.toBeInTheDocument();
  });

  it("gives up on a status request that never returns and stays usable", async () => {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    server.use(
      http.post(
        "*/v1/browser/project/status",
        () => new Promise<never>(() => undefined),
      ),
    );
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const { store } = render(<BrowserToggleButton chatId={chatId} />, {
        preloadedState: { chat },
      });
      act(() => {
        store.dispatch(openTab(`chat:${chatId}`));
      });
      fireEvent.click(screen.getByRole("button", { name: "Show browser" }));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1000);
      });
      expect(store.getState().browserViews.showingChats).toEqual([chatId]);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10_000);
      });
      expect(screen.getByRole("alert")).toHaveTextContent(
        STATUS_REFRESH_FAILED,
      );
      expect(store.getState().browserViews.showingChats).toEqual([]);
      expect(
        screen.getByRole("button", { name: "Show browser" }),
      ).toBeEnabled();
    } finally {
      vi.useRealTimers();
    }
  });
});
