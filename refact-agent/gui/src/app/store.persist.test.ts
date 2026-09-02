import { persistStore } from "redux-persist";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { setUpStore } from "./store";
import { createChatWithId } from "../features/Chat/Thread";
import {
  DOCK_MAX_WIDTH,
  DOCK_MIN_WIDTH,
  makeSurfaceKey,
  normalizeWorkspaceDock,
  openTab,
  setDockOpen,
  setDockSection,
  setDockWidth,
  setPanelsForced,
} from "../features/Workspace";

const WORKSPACE_PERSIST_KEY = "persist:workspace";

function readPersistedWorkspace(): Record<string, string> | null {
  const raw = localStorage.getItem(WORKSPACE_PERSIST_KEY);
  return raw ? (JSON.parse(raw) as Record<string, string>) : null;
}

function writePersistedWorkspace(entries: Record<string, unknown>) {
  localStorage.setItem(
    WORKSPACE_PERSIST_KEY,
    JSON.stringify(
      Object.fromEntries(
        Object.entries(entries).map(([key, value]) => [
          key,
          JSON.stringify(value),
        ]),
      ),
    ),
  );
}

async function flushPersist() {
  await new Promise((resolve) => setTimeout(resolve, 50));
}

function makePersistedStore() {
  const store = setUpStore();
  persistStore(store);
  return store;
}

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  localStorage.clear();
});

describe("workspace dock persistence", () => {
  it("persists only the dock slice of workspace state", async () => {
    const store = makePersistedStore();
    store.dispatch(createChatWithId({ id: "chat-a", title: "Chat Alpha" }));
    store.dispatch(openTab(makeSurfaceKey("chat", "chat-a")));
    store.dispatch(setDockSection("git"));
    store.dispatch(setDockWidth(320));
    store.dispatch(setDockOpen(true));
    store.dispatch(setPanelsForced(true));

    await flushPersist();

    const persisted = readPersistedWorkspace();
    expect(persisted).not.toBeNull();

    const persistedKeys = Object.keys(persisted ?? {}).filter(
      (key) => key !== "_persist",
    );
    expect(persistedKeys).toEqual(["dock"]);
    expect(JSON.parse(persisted?.dock ?? "null")).toEqual({
      open: true,
      width: 320,
      section: "git",
    });
  });

  it("does not persist tabs, groups, or panelsForced", async () => {
    const store = makePersistedStore();
    store.dispatch(createChatWithId({ id: "chat-a", title: "Chat Alpha" }));
    store.dispatch(openTab(makeSurfaceKey("chat", "chat-a")));
    store.dispatch(setPanelsForced(true));

    await flushPersist();

    const persisted = readPersistedWorkspace() ?? {};
    expect(persisted.tabs).toBeUndefined();
    expect(persisted.groups).toBeUndefined();
    expect(persisted.activeTabId).toBeUndefined();
    expect(persisted.panelsForced).toBeUndefined();
    expect(persisted.contextChatByTab).toBeUndefined();
  });

  it("keeps non-persisted workspace state working after a dock update", async () => {
    const store = makePersistedStore();
    const surfaceKey = makeSurfaceKey("chat", "chat-a");
    store.dispatch(createChatWithId({ id: "chat-a", title: "Chat Alpha" }));
    store.dispatch(openTab(surfaceKey));
    store.dispatch(setDockWidth(300));

    await flushPersist();

    expect(store.getState().workspace.tabs).toEqual([surfaceKey]);
    expect(store.getState().workspace.activeTabId).toBe(surfaceKey);
    expect(store.getState().workspace.dock?.width).toBe(300);
  });
});

describe("workspace dock migration", () => {
  it("normalizes a persisted dock with an out-of-range width", () => {
    expect(normalizeWorkspaceDock({ open: true, width: 10_000 })).toEqual({
      open: true,
      width: DOCK_MAX_WIDTH,
      section: "files",
    });
    expect(normalizeWorkspaceDock({ open: false, width: 1 })).toEqual({
      open: false,
      width: DOCK_MIN_WIDTH,
      section: "files",
    });
  });

  it("normalizes a persisted dock with an unknown section", () => {
    expect(
      normalizeWorkspaceDock({
        open: true,
        width: 300,
        section: "terminal" as never,
      }),
    ).toEqual({ open: true, width: 300, section: "files" });
  });

  it("fills defaults for a partial persisted dock", () => {
    expect(normalizeWorkspaceDock({})).toEqual({
      open: true,
      width: 280,
      section: "files",
    });
    expect(normalizeWorkspaceDock(undefined)).toEqual({
      open: true,
      width: 280,
      section: "files",
    });
    expect(normalizeWorkspaceDock(null)).toEqual({
      open: true,
      width: 280,
      section: "files",
    });
  });

  it("rehydrates a legacy persisted dock through the migration", async () => {
    writePersistedWorkspace({
      dock: { open: false, width: 9_999, section: "terminal" },
      _persist: { version: -1, rehydrated: false },
    });

    const store = setUpStore();
    await flushPersist();

    const dock = normalizeWorkspaceDock(store.getState().workspace.dock);
    expect(dock.width).toBeLessThanOrEqual(DOCK_MAX_WIDTH);
    expect(dock.width).toBeGreaterThanOrEqual(DOCK_MIN_WIDTH);
    expect(["files", "git", "agents", "tasks"]).toContain(dock.section);
  });
});
