import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { setUpStore } from "../../app/store";
import { browserApi } from "../../services/refact/browser";
import { server } from "../../utils/mockServer";
import { createDefaultChatState } from "../../utils/test-utils";
import { displayInfoForSurface } from "../Workspace/tabDisplay";
import {
  openTab,
  selectBrowserViewOwners,
  showBrowserTabForChat,
} from "../Workspace/workspaceSlice";
import {
  browserTabTitle,
  browserUrlLabel,
  mostRecentChatTab,
} from "./browserTabTitle";
import {
  openBrowserTabForChat,
  selectProjectBrowserStatus,
} from "./browserViews";
import { applyChatEvent } from "../Chat/Thread/actions";

const tab = (tabId: string, fields: Record<string, unknown> = {}) => ({
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

describe("browser tab titles", () => {
  it("prefers the tool name, then the page title, then host and path", () => {
    expect(
      browserTabTitle({
        name: "Docs",
        title: "Guide",
        url: "https://docs.test/guide",
      }),
    ).toBe("Docs");
    expect(
      browserTabTitle({ name: " ", title: "Guide", url: "https://x.test/" }),
    ).toBe("Guide");
    expect(
      browserTabTitle({ name: null, title: "", url: "https://x.test/a/b?q=1" }),
    ).toBe("x.test/a/b");
    expect(browserTabTitle({ url: "about:blank" })).toBe("New tab");
    expect(browserTabTitle(undefined)).toBe("Browser tab");
    expect(browserUrlLabel(`https://x.test/${"a".repeat(80)}`)).toHaveLength(
      48,
    );
  });

  it("titles browser tab surfaces from the tab metadata", () => {
    expect(
      displayInfoForSurface(
        "browser:tab-1",
        new Map(),
        {},
        new Map(),
        new Map([["tab-1", { title: "", url: "https://x.test/page" }]]),
      ),
    ).toMatchObject({ title: "x.test/page", kind: "browser" });
    expect(
      displayInfoForSurface("browser:tab-1", new Map(), {}, new Map()),
    ).toMatchObject({ title: "Browser tab", kind: "browser" });
  });

  it("picks the chat's own tab by last use, like the engine", () => {
    expect(
      mostRecentChatTab(
        [
          tab("old", { last_chat: "c", last_used_ms: 1 }),
          tab("recent", { last_chat: "c", last_used_ms: 3 }),
          tab("opened", { opened_by_chat: "c", last_used_ms: 5 }),
          tab("other", { last_chat: "d", last_used_ms: 9 }),
        ],
        "c",
      )?.tab_id,
    ).toBe("recent");
    expect(
      mostRecentChatTab(
        [
          tab("opened", { opened_by_chat: "c", last_used_ms: 5 }),
          tab("other", { last_chat: "d", last_used_ms: 9 }),
        ],
        "c",
      ),
    ).toBeUndefined();
    expect(mostRecentChatTab([tab("other")], "c")).toBeUndefined();
  });

  it("gives a tab opened by A and then used by B to B", () => {
    const tabs = [
      tab("b-own", { opened_by_chat: "b", last_chat: "b", last_used_ms: 1 }),
      tab("shared", { opened_by_chat: "a", last_chat: "b", last_used_ms: 7 }),
      tab("b-opened", { opened_by_chat: "b", last_used_ms: 9 }),
    ];
    expect(mostRecentChatTab(tabs, "b")?.tab_id).toBe("shared");
    expect(mostRecentChatTab(tabs, "a")).toBeUndefined();
  });
});

describe("browser workspace middleware", () => {
  it("prunes a view only after its tab is missing from two live statuses", async () => {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    const store = setUpStore({ chat });
    store.dispatch(openTab(`chat:${chatId}`));
    store.dispatch(showBrowserTabForChat({ chatId, tabId: "gone" }));
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json({
          project_id: "p",
          running: true,
          connected: true,
          runtime_id: "r",
          tabs: [tab("other")],
        }),
      ),
    );
    const poll = () =>
      store
        .dispatch(
          browserApi.endpoints.projectBrowserStatus.initiate(undefined, {
            forceRefetch: true,
          }),
        )
        .unwrap();
    await poll();
    expect(selectBrowserViewOwners(store.getState())).toEqual({
      gone: chatId,
    });
    await poll();
    expect(selectBrowserViewOwners(store.getState())).toEqual({});
  });

  it("opens next to the focused chat when no chat is given", () => {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    const store = setUpStore({ chat });
    store.dispatch(openTab(`chat:${chatId}`));
    store.dispatch(openBrowserTabForChat({ tabId: "tab-1" }));
    expect(selectBrowserViewOwners(store.getState())).toEqual({
      "tab-1": chatId,
    });
  });

  it("waits for the chat snapshot before opening", () => {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    chat.threads[chatId].snapshot_received = false;
    const store = setUpStore({ chat });
    store.dispatch(openTab(`chat:${chatId}`));
    store.dispatch(openBrowserTabForChat({ chatId, tabId: "tab-1" }));
    expect(selectBrowserViewOwners(store.getState())).toEqual({});
  });
});

describe("browser workspace middleware: busy and conflict statuses", () => {
  const live = (tabs: ReturnType<typeof tab>[], fields = {}) => ({
    project_id: "p",
    running: true,
    connected: true,
    runtime_id: "r",
    active_tab: null,
    tabs,
    ...fields,
  });

  function storeWithViews(tabIds: string[]) {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    const store = setUpStore({ chat });
    store.dispatch(openTab(`chat:${chatId}`));
    for (const tabId of tabIds)
      store.dispatch(showBrowserTabForChat({ chatId, tabId }));
    const poll = () =>
      store
        .dispatch(
          browserApi.endpoints.projectBrowserStatus.initiate(undefined, {
            forceRefetch: true,
          }),
        )
        .unwrap();
    return { store, chatId, poll };
  }

  it("never prunes views on busy statuses and keeps the last live tab list", async () => {
    const { store, chatId, poll } = storeWithViews(["t1"]);
    let body: Record<string, unknown> = live([tab("t1", { title: "Docs" })]);
    server.use(
      http.post("*/v1/browser/project/status", () => HttpResponse.json(body)),
    );
    await poll();
    body = live([], { busy: true });
    await poll();
    await poll();
    await poll();
    expect(selectBrowserViewOwners(store.getState())).toEqual({ t1: chatId });
    const status = selectProjectBrowserStatus(store.getState());
    expect(status?.busy).toBe(true);
    expect(status?.tabs.map((item) => item.tab_id)).toEqual(["t1"]);
  });

  it("counts a mutation's status response as a single miss", async () => {
    const { store, chatId, poll } = storeWithViews(["t1", "t2"]);
    server.use(
      http.post("*/v1/browser/project/navigate", () =>
        HttpResponse.json(live([tab("t1")])),
      ),
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json(live([tab("t1")])),
      ),
    );
    await store
      .dispatch(
        browserApi.endpoints.projectBrowserNavigate.initiate({
          op: "reload",
          tab_id: "t1",
        }),
      )
      .unwrap();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(selectBrowserViewOwners(store.getState())).toEqual({
      t1: chatId,
      t2: chatId,
    });
    await poll();
    expect(selectBrowserViewOwners(store.getState())).toEqual({ t1: chatId });
  });

  it("opens a deferred view once the chat snapshot arrives", () => {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    chat.threads[chatId].snapshot_received = false;
    const store = setUpStore({ chat });
    store.dispatch(openTab(`chat:${chatId}`));
    store.dispatch(openBrowserTabForChat({ chatId, tabId: "tab-1" }));
    expect(selectBrowserViewOwners(store.getState())).toEqual({});
    store.dispatch(
      applyChatEvent({
        chat_id: chatId,
        seq: "1",
        type: "snapshot",
        thread: {
          id: chatId,
          title: "Test",
          model: "test-model",
          mode: "AGENT",
          tool_use: "agent",
          boost_reasoning: false,
          context_tokens_cap: null,
          include_project_info: true,
          checkpoints_enabled: true,
          is_title_generated: false,
        },
        runtime: {
          state: "idle",
          paused: false,
          error: null,
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
        },
        messages: [],
        background_agents: [],
      }),
    );
    expect(selectBrowserViewOwners(store.getState())).toEqual({
      "tab-1": chatId,
    });
  });
});
