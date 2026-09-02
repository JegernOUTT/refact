import type { ReactNode } from "react";
import { Provider } from "react-redux";
import { renderHook } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";

import { type AppStore, setUpStore } from "../../app/store";
import type { BackgroundAgentSummary } from "../../services/refact";
import { createDefaultChatState } from "../../utils/test-utils";
import {
  agentsSectionUserClosed,
  autoOpenRequested,
} from "../AgentsPanel/agentsPanelSlice";
import { applyChatEvent, createChatWithId } from "../Chat/Thread";
import {
  openTab,
  setActiveTab,
  setDockOpen,
  setDockSection,
} from "./workspaceSlice";
import { makeSurfaceKey } from "./surfaceKey";
import { useAgentsSidebarAutoOpen } from "./useAgentsSidebarAutoOpen";

const chatId = "chat-a";
const originalMatchMedia = window.matchMedia;

function agent(
  id: string,
  overrides: Partial<BackgroundAgentSummary> = {},
): BackgroundAgentSummary {
  return {
    agent_id: id,
    parent_chat_id: chatId,
    child_chat_id: `${id}-child`,
    kind: "subagent",
    status: "running",
    title: id,
    progress: null,
    step_count: 0,
    last_activity: "2026-08-31T10:00:00Z",
    target_files: [],
    edited_files: [],
    diff_summary: null,
    conflict_summary: null,
    result_summary: null,
    error: null,
    started_at: null,
    finished_at: null,
    change_seq: 1,
    ...overrides,
  };
}

function mockNarrow(narrow: boolean) {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn(
      (query: string): MediaQueryList => ({
        matches: narrow && query === "(max-width: 767px)",
        media: query,
        onchange: null,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        addListener: vi.fn(),
        removeListener: vi.fn(),
        dispatchEvent: vi.fn(),
      }),
    ),
  });
}

function baseChatState() {
  const chat = createDefaultChatState();
  const [sourceRuntime] = Object.values(chat.threads);
  chat.current_thread_id = chatId;
  chat.open_thread_ids = [chatId];
  chat.threads = {
    [chatId]: {
      ...sourceRuntime,
      thread: { ...sourceRuntime.thread, id: chatId },
      background_agents: {},
    },
  };
  return chat;
}

function createAutoOpenStore(): AppStore {
  const store = setUpStore({ chat: baseChatState() });
  store.dispatch(createChatWithId({ id: chatId, title: "Chat Alpha" }));
  store.dispatch(openTab(makeSurfaceKey("chat", chatId)));
  store.dispatch(setActiveTab(makeSurfaceKey("chat", chatId)));
  return store;
}

function renderAutoOpen(store: AppStore) {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  return renderHook(() => useAgentsSidebarAutoOpen(), { wrapper });
}

let seq = 0;

function emitAgent(store: AppStore, summary: BackgroundAgentSummary) {
  seq += 1;
  act(() => {
    store.dispatch(
      applyChatEvent({
        chat_id: chatId,
        seq: String(seq),
        type: "background_agent_updated",
        agent: summary,
      }),
    );
  });
}

function startAgent(store: AppStore, id = "agent-1") {
  emitAgent(store, agent(id, { change_seq: 1 }));
}

function finishAgent(store: AppStore, id = "agent-1") {
  emitAgent(store, agent(id, { status: "completed", change_seq: 2 }));
}

describe("useAgentsSidebarAutoOpen", () => {
  afterEach(() => {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: originalMatchMedia,
    });
    vi.restoreAllMocks();
  });

  it("opens the agents dock section when the first agent starts", () => {
    mockNarrow(false);
    const store = createAutoOpenStore();
    store.dispatch(setDockOpen(false));
    renderAutoOpen(store);

    startAgent(store);

    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "agents",
    });
  });

  it("does not auto-open on narrow viewports", () => {
    mockNarrow(true);
    const store = createAutoOpenStore();
    store.dispatch(setDockOpen(false));
    renderAutoOpen(store);

    startAgent(store);

    expect(store.getState().workspace.dock?.open).toBe(false);
  });

  it("does not hijack an already open dock section", () => {
    mockNarrow(false);
    const store = createAutoOpenStore();
    store.dispatch(setDockSection("files"));
    store.dispatch(setDockOpen(true));
    renderAutoOpen(store);

    startAgent(store);

    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "files",
    });
  });

  it("respects an explicit user close for the focused chat", () => {
    mockNarrow(false);
    const store = createAutoOpenStore();
    store.dispatch(setDockOpen(false));
    store.dispatch(agentsSectionUserClosed(chatId));
    renderAutoOpen(store);

    startAgent(store);

    expect(store.getState().workspace.dock?.open).toBe(false);
  });

  it("closes the dock again once the auto-opened run finishes", () => {
    mockNarrow(false);
    const store = createAutoOpenStore();
    store.dispatch(setDockOpen(false));
    renderAutoOpen(store);

    startAgent(store);
    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "agents",
    });
    expect(store.getState().agentsPanel.autoOpenedFor).toBe(chatId);

    finishAgent(store);

    expect(store.getState().workspace.dock?.open).toBe(false);
  });

  it("keeps a manually opened agents dock open when agents finish", () => {
    mockNarrow(false);
    const store = createAutoOpenStore();
    store.dispatch(setDockSection("agents"));
    store.dispatch(setDockOpen(true));
    renderAutoOpen(store);

    startAgent(store);
    expect(store.getState().agentsPanel.autoOpenedFor).toBeNull();

    finishAgent(store);

    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "agents",
    });
  });

  it("leaves a different dock section untouched when agents finish", () => {
    mockNarrow(false);
    const store = createAutoOpenStore();
    store.dispatch(setDockOpen(false));
    renderAutoOpen(store);

    startAgent(store);
    act(() => {
      store.dispatch(autoOpenRequested(chatId));
      store.dispatch(setDockSection("files"));
    });

    finishAgent(store);

    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "files",
    });
  });

  it("does not auto-open on the first render for an already running agent", () => {
    mockNarrow(false);
    const store = createAutoOpenStore();
    store.dispatch(setDockOpen(false));
    startAgent(store);

    renderAutoOpen(store);

    expect(store.getState().workspace.dock?.open).toBe(false);
  });
});
