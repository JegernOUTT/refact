import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { server } from "../../utils/mockServer";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../utils/test-utils";
import {
  openTab,
  selectBrowserViewOwners,
  selectWorkspaceDock,
  showBrowserTabForChat,
} from "../Workspace/workspaceSlice";
import {
  BROWSER_DOCK_TASK_CHAT_NOTE,
  BrowserDockPanel,
} from "./BrowserDockPanel";
import { BrowserRail } from "./BrowserRail";

const tab = (
  tabId: string,
  fields: Partial<{
    url: string;
    title: string;
    name: string | null;
    opened_by_chat: string | null;
    last_chat: string | null;
  }>,
) => ({
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

type SetupTabs = (ids: { chatId: string; otherId: string }) => TestTab[];

type TestTab = ReturnType<typeof tab>;

function setup(makeTabs: SetupTabs) {
  const chat = createDefaultChatState();
  const other = createDefaultChatState();
  const chatId = chat.current_thread_id;
  const otherId = other.current_thread_id;
  chat.threads[chatId].thread.title = "Focused chat";
  other.threads[otherId].thread.title = "Other chat";
  const requests: unknown[] = [];
  let currentTabs = makeTabs({ chatId, otherId });
  const status = () => ({
    project_id: "test-project",
    running: true,
    connected: true,
    runtime_id: "runtime-a",
    active_tab: null,
    tabs: currentTabs,
  });
  server.use(
    http.post("*/v1/browser/project/status", () => HttpResponse.json(status())),
    http.post("*/v1/browser/project/tab", async ({ request }) => {
      const body = (await request.json()) as { op: string; tab_id?: string };
      requests.push(body);
      if (body.op === "close")
        currentTabs = currentTabs.filter((item) => item.tab_id !== body.tab_id);
      if (body.op === "new") {
        currentTabs = [...currentTabs, tab("tab-new", {})];
        return HttpResponse.json({ ...status(), new_tab_id: "tab-new" });
      }
      return HttpResponse.json(status());
    }),
  );
  const view = render(<BrowserDockPanel />, {
    preloadedState: {
      chat: {
        ...chat,
        open_thread_ids: [chatId, otherId],
        threads: { ...chat.threads, ...other.threads },
      },
    },
  });
  view.store.dispatch(openTab(`chat:${chatId}`));
  return { ...view, chatId, otherId, requests };
}

describe("Browser dock panel", () => {
  it("lists tabs with their owning chat and marks the one shown here", async () => {
    const { store, chatId } = setup(({ chatId, otherId }) => [
      tab("tab-a", {
        url: "https://docs.test/guide",
        name: "Docs",
        last_chat: chatId,
      }),
      tab("tab-b", {
        url: "https://example.test/page",
        title: "Example",
        opened_by_chat: otherId,
      }),
    ]);
    store.dispatch(showBrowserTabForChat({ chatId, tabId: "tab-a" }));
    const list = await screen.findByRole("list", { name: "Browser tabs" });
    const rows = within(list).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(within(rows[0]).getByText("Docs")).toBeInTheDocument();
    expect(within(rows[0]).getByText("Open here")).toBeInTheDocument();
    expect(within(rows[0]).getByText("Focused chat")).toBeInTheDocument();
    expect(within(rows[0]).getByText("docs.test/guide")).toBeInTheDocument();
    expect(within(rows[1]).getByText("Example")).toBeInTheDocument();
    expect(within(rows[1]).getByText("Other chat")).toBeInTheDocument();
    expect(within(rows[1]).queryByText("Open here")).not.toBeInTheDocument();
  });

  it("opens a row next to the focused chat, moving it from another chat", async () => {
    const { store, chatId, otherId } = setup(() => [
      tab("tab-b", { title: "Example" }),
    ]);
    store.dispatch(showBrowserTabForChat({ chatId: otherId, tabId: "tab-b" }));
    fireEvent.click(await screen.findByRole("button", { name: /^Example/ }));
    expect(selectBrowserViewOwners(store.getState())).toEqual({
      "tab-b": chatId,
    });
  });

  it("closes the Chromium tab and its view from the row action", async () => {
    const { store, chatId, requests } = setup(() => [
      tab("tab-a", { title: "Example" }),
    ]);
    store.dispatch(showBrowserTabForChat({ chatId, tabId: "tab-a" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "Close tab: Example" }),
    );
    await waitFor(() =>
      expect(requests).toEqual([{ op: "close", tab_id: "tab-a" }]),
    );
    await waitFor(() =>
      expect(selectBrowserViewOwners(store.getState())).toEqual({}),
    );
  });

  it("treats an already-gone tab as closed when the browser answers 410", async () => {
    const { store, chatId } = setup(() => [tab("tab-a", { title: "Example" })]);
    let statuses = 0;
    let gone = false;
    server.use(
      http.post("*/v1/browser/project/status", () => {
        statuses += 1;
        return HttpResponse.json({
          project_id: "test-project",
          running: true,
          connected: true,
          runtime_id: "runtime-a",
          active_tab: null,
          tabs: gone ? [] : [tab("tab-a", { title: "Example" })],
        });
      }),
      http.post("*/v1/browser/project/tab", () => {
        gone = true;
        return HttpResponse.json({ detail: "unknown tab" }, { status: 410 });
      }),
    );
    store.dispatch(showBrowserTabForChat({ chatId, tabId: "tab-a" }));
    const close = await screen.findByRole("button", {
      name: "Close tab: Example",
    });
    const before = statuses;
    fireEvent.click(close);
    await waitFor(() =>
      expect(selectBrowserViewOwners(store.getState())).toEqual({}),
    );
    await waitFor(() => expect(statuses).toBeGreaterThan(before));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("keeps showing a refused close as an error", async () => {
    const { store, chatId } = setup(() => [tab("tab-a", { title: "Example" })]);
    server.use(
      http.post("*/v1/browser/project/tab", () =>
        HttpResponse.json(
          { detail: "the browser refused to close the tab" },
          { status: 502 },
        ),
      ),
    );
    store.dispatch(showBrowserTabForChat({ chatId, tabId: "tab-a" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "Close tab: Example" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "the browser refused to close the tab",
    );
    expect(selectBrowserViewOwners(store.getState())).toEqual({
      "tab-a": chatId,
    });
  });

  it("creates a tab for the focused chat and opens it next to that chat", async () => {
    const { store, chatId, requests } = setup(() => []);
    await screen.findByText(/No browser tabs yet/);
    fireEvent.click(screen.getByRole("button", { name: "New tab" }));
    await waitFor(() =>
      expect(requests).toEqual([{ op: "new", chat_id: chatId }]),
    );
    await waitFor(() =>
      expect(selectBrowserViewOwners(store.getState())).toEqual({
        "tab-new": chatId,
      }),
    );
  });

  it("offers to start a browser that is not running, without launching it itself", async () => {
    let starts = 0;
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json({
          project_id: "test-project",
          running: false,
          connected: false,
          tabs: [],
        }),
      ),
      http.post("*/v1/browser/project/start", () => {
        starts += 1;
        return HttpResponse.json(
          { detail: "Chrome is not installed" },
          { status: 500 },
        );
      }),
    );
    render(<BrowserDockPanel />);
    fireEvent.click(
      await screen.findByRole("button", { name: "Start browser" }),
    );
    expect(
      await screen.findByText("Chrome is not installed"),
    ).toBeInTheDocument();
    expect(starts).toBe(1);
    fireEvent.click(screen.getByRole("button", { name: "Start browser" }));
    await waitFor(() => expect(starts).toBe(2));
  });
});

describe("Browser dock panel in a task workspace", () => {
  it("adds tabs for the focused task chat and explains why they are not shown", async () => {
    const chat = createDefaultChatState();
    const requests: unknown[] = [];
    const status = {
      project_id: "test-project",
      running: true,
      connected: true,
      runtime_id: "runtime-a",
      active_tab: null,
      tabs: [tab("tab-a", { url: "https://example.test/a" })],
    };
    server.use(
      http.post("*/v1/browser/project/status", () => HttpResponse.json(status)),
      http.post("*/v1/browser/project/tab", async ({ request }) => {
        requests.push(await request.json());
        return HttpResponse.json({ ...status, new_tab_id: "tab-new" });
      }),
    );
    render(<BrowserDockPanel />, {
      preloadedState: {
        chat,
        pages: [
          { name: "chat" },
          {
            name: "task agent",
            taskId: "task-1",
            agentId: "agent-1",
            chatId: "task-chat",
          },
        ],
      },
    });
    expect(
      await screen.findByText(BROWSER_DOCK_TASK_CHAT_NOTE),
    ).toBeInTheDocument();
    const list = await screen.findByRole("list", { name: "Browser tabs" });
    expect(
      within(list).getByRole("button", { name: /^example\.test/ }),
    ).toBeDisabled();
    const newTab = screen.getByRole("button", { name: "New tab" });
    expect(newTab).toBeEnabled();
    fireEvent.click(newTab);
    await waitFor(() =>
      expect(requests).toEqual([{ op: "new", chat_id: "task-chat" }]),
    );
  });
});

describe("Browser rail", () => {
  it("toggles the browser dock section", () => {
    const { store } = render(<BrowserRail />);
    const button = screen.getByRole("button", { name: "Browser" });
    fireEvent.click(button);
    expect(selectWorkspaceDock(store.getState())).toMatchObject({
      open: true,
      section: "browser",
    });
    fireEvent.click(button);
    expect(selectWorkspaceDock(store.getState()).open).toBe(false);
  });
});
