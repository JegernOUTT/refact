import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it } from "vitest";
import { server } from "../../utils/mockServer";
import type { AppStore } from "../../app/store";
import type { ChatEventEnvelope } from "../../services/refact/chatSubscription";
import { applyChatEvent } from "../Chat/Thread/actions";
import { collectTabIds, findLeafByTab } from "../ChatPanes/panesTree";
import { createDefaultChatState, render } from "../../utils/test-utils";
import {
  projectBrowserOpenHint,
  useProjectBrowserAutoOpen,
} from "./useProjectBrowserAutoOpen";
import {
  closeTab,
  focusPane,
  openTab,
  selectBrowserViewOwners,
  selectChatHasBrowserTab,
  selectWorkspaceFocusedChatId,
  splitPaneWithSurface,
} from "./workspaceSlice";

function Subscriber() {
  useProjectBrowserAutoOpen();
  return null;
}

function twoChatState() {
  const first = createDefaultChatState();
  const second = createDefaultChatState();
  const a = first.current_thread_id;
  const b = second.current_thread_id;
  return {
    a,
    b,
    chat: {
      ...first,
      current_thread_id: b,
      open_thread_ids: [a, b],
      threads: { ...first.threads, ...second.threads },
    },
  };
}

type WireMessage = Extract<
  ChatEventEnvelope,
  { type: "message_added" }
>["message"];

const wireToolMessage = (
  toolCallId: string,
  hint: Record<string, unknown>,
): WireMessage =>
  ({
    role: "tool",
    tool_call_id: toolCallId,
    message_id: toolCallId,
    content: "done",
    tool_failed: false,
    browser_project: hint,
  }) as WireMessage;

const browserHint = (tabId: string | null, tabName: string | null) => ({
  open_browser_tab: true,
  runtime_id: "runtime-a",
  tab_id: tabId,
  tab_name: tabName,
});

const toolResult = (
  chatId: string,
  toolCallId: string,
  tabId: string | null,
  type: "message_added" | "message_updated" = "message_added",
  tabName: string | null = null,
) => {
  const message = wireToolMessage(toolCallId, browserHint(tabId, tabName));
  return applyChatEvent(
    type === "message_added"
      ? { chat_id: chatId, seq: toolCallId, type, index: 0, message }
      : {
          chat_id: chatId,
          seq: toolCallId,
          type,
          message_id: toolCallId,
          message,
        },
  );
};

const snapshotWith = (chatId: string, messages: WireMessage[]) =>
  applyChatEvent({
    chat_id: chatId,
    seq: "snapshot",
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
    messages,
    background_agents: [],
  });

const snapshotChunk = (chatId: string, messages: WireMessage[]) =>
  applyChatEvent({
    chat_id: chatId,
    seq: "chunk",
    type: "snapshot_chunk",
    chunk_index: 0,
    chunk_count: 1,
    messages,
  });

beforeEach(() => {
  server.use(
    http.post("*/v1/browser/project/status", () =>
      HttpResponse.json({
        project_id: "p",
        running: false,
        connected: false,
        tabs: [],
      }),
    ),
  );
});

function setup() {
  const { a, b, chat } = twoChatState();
  const { store } = render(<Subscriber />, { preloadedState: { chat } });
  store.dispatch(openTab(`chat:${a}`));
  store.dispatch(openTab(`chat:${b}`));
  return { a, b, store };
}

const shown = (store: AppStore, chatId: string, tabId: string) =>
  selectChatHasBrowserTab(store.getState(), chatId, tabId);

describe("project browser auto-open", () => {
  it("parses only shared-browser tool hints with a tab id", () => {
    const event = toolResult("chat", "call-1", "tab-1").payload;
    expect(projectBrowserOpenHint(event)).toEqual({
      toolCallId: "call-1",
      tabId: "tab-1",
      tabName: null,
    });
    const legacy = applyChatEvent({
      chat_id: "chat",
      seq: "1",
      type: "message_added",
      index: 0,
      message: wireToolMessage("call-2", {
        open_browser_tab: true,
        decision_required: true,
      }),
    }).payload;
    expect(projectBrowserOpenHint(legacy)).toBeUndefined();
  });

  it("reads the hint from the flattened wire message, not from extra", () => {
    const nested = applyChatEvent({
      chat_id: "chat",
      seq: "1",
      type: "message_added",
      index: 0,
      message: {
        role: "tool",
        tool_call_id: "call-3",
        content: "done",
        extra: { browser_project: browserHint("tab-1", null) },
      },
    }).payload;
    expect(projectBrowserOpenHint(nested)).toBeUndefined();
    const { b, store } = setup();
    store.dispatch(toolResult(b, "call-1", "tab-1", "message_added", "Docs"));
    expect(shown(store, b, "tab-1")).toBe(true);
    expect(store.getState().browserViews.names).toEqual({ "tab-1": "Docs" });
  });

  it("records snapshot hints as history without reopening their views", () => {
    const { b, store } = setup();
    store.dispatch(
      snapshotWith(b, [
        wireToolMessage("call-old", browserHint("tab-old", "Old docs")),
      ]),
    );
    expect(shown(store, b, "tab-old")).toBe(false);
    expect(store.getState().browserViews.names).toEqual({
      "tab-old": "Old docs",
    });
    store.dispatch(toolResult(b, "call-old", "tab-old", "message_updated"));
    expect(shown(store, b, "tab-old")).toBe(false);
    store.dispatch(toolResult(b, "call-new", "tab-old"));
    expect(shown(store, b, "tab-old")).toBe(true);
  });

  it("opens next to the visible chat as a row split, chat left and browser right", () => {
    const { b, store } = setup();
    store.dispatch(toolResult(b, "call-1", "tab-1"));
    const workspace = store.getState().workspace;
    const group = workspace.groups[`chat:${b}`];
    expect(workspace.activeTabId).toBe(`chat:${b}`);
    expect(group?.root.kind).toBe("split");
    if (group?.root.kind !== "split") return;
    expect(group.root.dir).toBe("row");
    expect(group.root.children.map((child) => collectTabIds(child))).toEqual([
      [`chat:${b}`],
      ["browser:tab-1"],
    ]);
    expect(group.focusedLeafId).toBe(
      findLeafByTab(group.root, `chat:${b}`)?.id,
    );
  });

  it("adds a background chat's view to its own group without switching the visible group", () => {
    const { a, b, store } = setup();
    const visibleBefore = store.getState().workspace.activeTabId;
    store.dispatch(toolResult(a, "call-1", "tab-1"));
    const workspace = store.getState().workspace;
    expect(shown(store, a, "tab-1")).toBe(true);
    expect(workspace.activeTabId).not.toBe(`chat:${a}`);
    const visible = workspace.groups[workspace.activeTabId ?? ""];
    expect(visible && collectTabIds(visible.root)).toEqual([`chat:${b}`]);
    expect(visibleBefore).toBeTruthy();
  });

  it("joins a second tab to the existing browser leaf", () => {
    const { b, store } = setup();
    store.dispatch(toolResult(b, "call-1", "tab-1"));
    store.dispatch(toolResult(b, "call-2", "tab-2"));
    const group = store.getState().workspace.groups[`chat:${b}`];
    expect(group?.root.kind === "split" && group.root.children.length).toBe(2);
    expect(group && findLeafByTab(group.root, "browser:tab-2")?.tabIds).toEqual(
      ["browser:tab-1", "browser:tab-2"],
    );
  });

  it("reopens after a close on a new tool call but never for the same call", () => {
    const { b, store } = setup();
    store.dispatch(toolResult(b, "call-1", "tab-1"));
    expect(shown(store, b, "tab-1")).toBe(true);
    store.dispatch(closeTab("browser:tab-1"));
    expect(shown(store, b, "tab-1")).toBe(false);
    store.dispatch(toolResult(b, "call-1", "tab-1", "message_updated"));
    expect(shown(store, b, "tab-1")).toBe(false);
    store.dispatch(toolResult(b, "call-2", "tab-1"));
    expect(shown(store, b, "tab-1")).toBe(true);
  });

  it("keeps the tool-provided name for the tab title", () => {
    const { b, store } = setup();
    store.dispatch(toolResult(b, "call-1", "tab-1", "message_added", "Docs"));
    expect(store.getState().browserViews.names).toEqual({ "tab-1": "Docs" });
    expect(selectBrowserViewOwners(store.getState())).toEqual({
      "tab-1": b,
    });
  });

  it("waits for the chat snapshot before handling a hint", () => {
    const { a, b, chat } = twoChatState();
    chat.threads[b].snapshot_received = false;
    const { store } = render(<Subscriber />, { preloadedState: { chat } });
    store.dispatch(openTab(`chat:${a}`));
    store.dispatch(openTab(`chat:${b}`));
    store.dispatch(toolResult(b, "call-1", "tab-1"));
    expect(shown(store, b, "tab-1")).toBe(false);
    expect(store.getState().browserViews.handledHints).toEqual({});
  });

  it("opens nothing for a hint without a tab id and dedupes it by tool call", () => {
    const { b, store } = setup();
    store.dispatch(toolResult(b, "call-1", null, "message_added", "Docs"));
    expect(selectBrowserViewOwners(store.getState())).toEqual({});
    expect(store.getState().browserViews.names).toEqual({});
    expect(store.getState().browserViews.handledHints).toEqual({
      [b]: ["call-1"],
    });
    store.dispatch(toolResult(b, "call-1", null, "message_updated"));
    expect(store.getState().browserViews.handledHints).toEqual({
      [b]: ["call-1"],
    });
    expect(selectBrowserViewOwners(store.getState())).toEqual({});
  });

  it("never moves a view out of the visible chat for a background chat's hint", () => {
    const { a, b, store } = setup();
    store.dispatch(toolResult(b, "call-1", "tab-1"));
    store.dispatch(toolResult(a, "call-2", "tab-1"));
    expect(selectBrowserViewOwners(store.getState())).toEqual({ "tab-1": b });
    expect(store.getState().workspace.activeTabId).toBe(`chat:${b}`);
  });

  it("keeps focus on the focused chat when a split neighbour opens a tab", () => {
    const { a, b, store } = setup();
    const workspace = store.getState().workspace;
    const key = workspace.activeTabId ?? "";
    const leaf = findLeafByTab(
      workspace.groups[key]?.root ?? {
        kind: "leaf",
        id: "x",
        tabIds: [],
        activeTabId: null,
      },
      `chat:${a}`,
    );
    if (!leaf) throw new Error("missing chat leaf");
    store.dispatch(
      splitPaneWithSurface({
        tabId: key,
        leafId: leaf.id,
        surfaceKey: `chat:${b}`,
        dir: "row",
        placement: "after",
      }),
    );
    const active = store.getState().workspace.activeTabId ?? "";
    const left = findLeafByTab(
      store.getState().workspace.groups[active]?.root ?? {
        kind: "leaf",
        id: "x",
        tabIds: [],
        activeTabId: null,
      },
      `chat:${a}`,
    );
    if (!left) throw new Error("missing left leaf");
    store.dispatch(focusPane({ tabId: active, leafId: left.id }));
    store.dispatch(toolResult(b, "call-1", "tab-1"));
    expect(store.getState().workspace.activeTabId).toBe(active);
    expect(selectWorkspaceFocusedChatId(store.getState())).toBe(a);
    expect(shown(store, b, "tab-1")).toBe(true);
  });

  it("drops hints from chats without a workspace view", () => {
    const { chat } = twoChatState();
    const id = chat.current_thread_id;
    const { store } = render(<Subscriber />, {
      preloadedState: { chat: { ...chat, open_thread_ids: [] } },
    });
    store.dispatch(toolResult(id, "call-1", "tab-1"));
    expect(selectBrowserViewOwners(store.getState())).toEqual({});
  });

  it("handles a snapshot's hints with a single store update", () => {
    const { b, store } = setup();
    let updates = 0;
    const unsubscribe = store.subscribe(() => {
      updates += 1;
    });
    store.dispatch(
      snapshotChunk(
        b,
        Array.from({ length: 300 }, (_, index) =>
          wireToolMessage(`old-${index}`, browserHint("tab-old", "Docs")),
        ),
      ),
    );
    unsubscribe();
    expect(updates).toBe(2);
    expect(store.getState().browserViews.handledHints[b]).toHaveLength(256);
    expect(store.getState().browserViews.names).toEqual({ "tab-old": "Docs" });
    expect(selectBrowserViewOwners(store.getState())).toEqual({});
  });

  it("keeps each chat's handled calls when another chat replays a large snapshot", () => {
    const { a, b, store } = setup();
    store.dispatch(toolResult(b, "call-1", "tab-1"));
    store.dispatch(closeTab("browser:tab-1"));
    store.dispatch(
      snapshotChunk(
        a,
        Array.from({ length: 300 }, (_, index) =>
          wireToolMessage(`old-${index}`, browserHint("tab-old", null)),
        ),
      ),
    );
    store.dispatch(toolResult(b, "call-1", "tab-1", "message_updated"));
    expect(selectBrowserViewOwners(store.getState())).toEqual({});
  });
});
