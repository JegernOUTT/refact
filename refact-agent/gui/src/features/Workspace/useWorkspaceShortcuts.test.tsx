import type { ReactNode } from "react";
import { Provider } from "react-redux";
import { fireEvent, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { setUpStore } from "../../app/store";
import { createChatWithId } from "../Chat/Thread";
import { updateConfig } from "../Config/configSlice";
import { setTerminalWorkbenchOpen } from "./TerminalPanel/terminalSlice";
import {
  openTab,
  setDockOpen,
  setDockSection,
  setPanelsForced,
} from "./workspaceSlice";
import { makeSurfaceKey } from "./surfaceKey";
import { useWorkspaceShortcuts } from "./useWorkspaceShortcuts";

function renderShortcuts() {
  const store = setUpStore();
  store.dispatch(createChatWithId({ id: "chat-a" }));
  store.dispatch(openTab(makeSurfaceKey("chat", "chat-a")));
  const wrapper = ({ children }: { children: ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  const view = renderHook(() => useWorkspaceShortcuts(), { wrapper });
  return { store, ...view };
}

describe("useWorkspaceShortcuts", () => {
  it("opens the dock and current chat terminal when the dock is closed", () => {
    const { store, rerender } = renderShortcuts();
    store.dispatch(setTerminalWorkbenchOpen({ chatId: "chat-a", open: true }));
    store.dispatch(setDockOpen(false));
    rerender();

    fireEvent.keyDown(window, { key: "j", ctrlKey: true });

    expect(store.getState().workspace.dock?.open).toBe(true);
    expect(store.getState().terminal.workbenchOpenByChat["chat-a"]).toBe(true);
  });

  it("toggles the current chat terminal when the dock is already open", () => {
    const { store } = renderShortcuts();

    fireEvent.keyDown(window, { key: "j", ctrlKey: true });
    expect(store.getState().terminal.workbenchOpenByChat["chat-a"]).toBe(true);

    fireEvent.keyDown(window, { key: "j", metaKey: true });
    expect(store.getState().terminal.workbenchOpenByChat["chat-a"]).toBe(false);
    expect(store.getState().workspace.dock?.open).toBe(true);
  });

  it("toggles web workspace chrome and selects visible dock sections", () => {
    const { store } = renderShortcuts();

    fireEvent.keyDown(window, { key: "b", ctrlKey: true });
    expect(store.getState().workspace.dock?.open).toBe(false);

    fireEvent.keyDown(window, { key: "B", metaKey: true });
    expect(store.getState().workspace.dock?.open).toBe(true);

    fireEvent.keyDown(window, { key: "2", ctrlKey: true });
    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "git",
    });

    fireEvent.keyDown(window, { key: "3", metaKey: true });
    expect(store.getState().workspace.dock?.section).toBe("tasks");
  });

  it("ignores the terminal shortcut when terminal capability is unavailable", () => {
    const { store, rerender } = renderShortcuts();
    store.dispatch(
      updateConfig({
        capabilities: {
          filesPanel: true,
          gitPanel: true,
          terminalPanel: false,
        },
      }),
    );
    store.dispatch(setDockOpen(false));
    rerender();

    fireEvent.keyDown(window, { key: "j", ctrlKey: true });

    expect(
      store.getState().terminal.workbenchOpenByChat["chat-a"],
    ).toBeUndefined();
    expect(store.getState().workspace.dock?.open).toBe(false);
  });

  it("ignores section shortcuts whose capability is unavailable", () => {
    const store = setUpStore({
      config: {
        host: "web",
        lspPort: 8001,
        themeProps: { appearance: "dark" },
        capabilities: { filesPanel: false, gitPanel: true },
      },
    });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <Provider store={store}>{children}</Provider>
    );
    store.dispatch(setDockSection("tasks"));
    renderHook(() => useWorkspaceShortcuts(), { wrapper });

    fireEvent.keyDown(window, { key: "1", ctrlKey: true });
    expect(store.getState().workspace.dock?.section).toBe("tasks");
    fireEvent.keyDown(window, { key: "2", ctrlKey: true });
    expect(store.getState().workspace.dock?.section).toBe("git");
  });

  it("supports terminal-only workspace shortcuts", () => {
    const { store, rerender } = renderShortcuts();
    store.dispatch(
      updateConfig({
        capabilities: {
          filesPanel: false,
          gitPanel: false,
          terminalPanel: true,
        },
      }),
    );
    store.dispatch(setDockOpen(false));
    rerender();

    fireEvent.keyDown(window, { key: "b", ctrlKey: true });
    expect(store.getState().workspace.dock?.open).toBe(true);

    store.dispatch(setDockOpen(false));
    rerender();
    fireEvent.keyDown(window, { key: "j", metaKey: true });
    expect(store.getState().workspace.dock?.open).toBe(true);
    expect(store.getState().terminal.workbenchOpenByChat["chat-a"]).toBe(true);

    fireEvent.keyDown(window, { key: "3", ctrlKey: true });
    expect(store.getState().workspace.dock?.section).toBe("files");
  });

  it("supports task shortcuts when workspace panels are forced", () => {
    const { store, rerender } = renderShortcuts();
    store.dispatch(
      updateConfig({
        capabilities: {
          filesPanel: false,
          gitPanel: false,
          terminalPanel: false,
        },
      }),
    );
    store.dispatch(setPanelsForced(true));
    store.dispatch(setDockOpen(false));
    rerender();

    fireEvent.keyDown(window, { key: "b", ctrlKey: true });
    expect(store.getState().workspace.dock?.open).toBe(true);

    fireEvent.keyDown(window, { key: "3", metaKey: true });
    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "tasks",
    });
  });

  it("gates files-only workspace shortcuts by target", () => {
    const { store, rerender } = renderShortcuts();
    store.dispatch(
      updateConfig({
        capabilities: {
          filesPanel: true,
          gitPanel: false,
          terminalPanel: false,
        },
      }),
    );
    store.dispatch(setDockSection("tasks"));
    store.dispatch(setDockOpen(false));
    rerender();

    fireEvent.keyDown(window, { key: "b", ctrlKey: true });
    fireEvent.keyDown(window, { key: "1", ctrlKey: true });
    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "files",
    });

    fireEvent.keyDown(window, { key: "3", ctrlKey: true });
    expect(store.getState().workspace.dock?.section).toBe("tasks");
    fireEvent.keyDown(window, { key: "1", ctrlKey: true });
    fireEvent.keyDown(window, { key: "2", ctrlKey: true });
    fireEvent.keyDown(window, { key: "j", ctrlKey: true });
    expect(store.getState().workspace.dock?.section).toBe("files");
    expect(
      store.getState().terminal.workbenchOpenByChat["chat-a"],
    ).toBeUndefined();
  });

  it("gates git-only workspace shortcuts by target", () => {
    const { store, rerender } = renderShortcuts();
    store.dispatch(
      updateConfig({
        capabilities: {
          filesPanel: false,
          gitPanel: true,
          terminalPanel: false,
        },
      }),
    );
    store.dispatch(setDockSection("tasks"));
    store.dispatch(setDockOpen(false));
    rerender();

    fireEvent.keyDown(window, { key: "b", metaKey: true });
    fireEvent.keyDown(window, { key: "2", metaKey: true });
    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "git",
    });

    fireEvent.keyDown(window, { key: "3", metaKey: true });
    expect(store.getState().workspace.dock?.section).toBe("tasks");
    fireEvent.keyDown(window, { key: "2", metaKey: true });
    fireEvent.keyDown(window, { key: "1", metaKey: true });
    fireEvent.keyDown(window, { key: "j", metaKey: true });
    expect(store.getState().workspace.dock?.section).toBe("git");
    expect(
      store.getState().terminal.workbenchOpenByChat["chat-a"],
    ).toBeUndefined();
  });

  it("ignores workspace shortcuts when no target is available", () => {
    const { store, rerender } = renderShortcuts();
    store.dispatch(
      updateConfig({
        capabilities: {
          filesPanel: false,
          gitPanel: false,
          terminalPanel: false,
        },
      }),
    );
    store.dispatch(setDockSection("tasks"));
    store.dispatch(setDockOpen(false));
    rerender();

    for (const key of ["b", "j", "1", "2", "3"]) {
      fireEvent.keyDown(window, { key, ctrlKey: true });
    }

    expect(store.getState().workspace.dock).toMatchObject({
      open: false,
      section: "tasks",
    });
    expect(
      store.getState().terminal.workbenchOpenByChat["chat-a"],
    ).toBeUndefined();
  });

  it("does not fire from editable controls, contenteditable regions, or xterm", () => {
    const { store } = renderShortcuts();
    const input = document.body.appendChild(document.createElement("input"));
    const editable = document.body.appendChild(document.createElement("div"));
    editable.contentEditable = "true";
    const terminal = document.body.appendChild(document.createElement("div"));
    terminal.className = "xterm";

    fireEvent.keyDown(input, { key: "j", ctrlKey: true });
    fireEvent.keyDown(editable, { key: "2", ctrlKey: true });
    fireEvent.keyDown(terminal, { key: "j", metaKey: true });

    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "files",
    });
    expect(
      store.getState().terminal.workbenchOpenByChat["chat-a"],
    ).toBeUndefined();

    input.remove();
    editable.remove();
    terminal.remove();
  });

  it("does not register workspace shortcuts for IDE hosts", () => {
    const { store, rerender } = renderShortcuts();
    store.dispatch(updateConfig({ host: "vscode" }));
    rerender();

    fireEvent.keyDown(window, { key: "b", ctrlKey: true });
    fireEvent.keyDown(window, { key: "j", ctrlKey: true });
    fireEvent.keyDown(window, { key: "2", ctrlKey: true });

    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "files",
    });
    expect(
      store.getState().terminal.workbenchOpenByChat["chat-a"],
    ).toBeUndefined();
  });

  it("does not register forced panel shortcuts for IDE hosts", () => {
    const { store, rerender } = renderShortcuts();
    store.dispatch(updateConfig({ host: "vscode" }));
    store.dispatch(setPanelsForced(true));
    store.dispatch(setDockSection("tasks"));
    store.dispatch(setDockOpen(false));
    rerender();

    for (const key of ["b", "j", "1", "2", "3"]) {
      fireEvent.keyDown(window, { key, ctrlKey: true });
    }

    expect(store.getState().workspace.dock).toMatchObject({
      open: false,
      section: "tasks",
    });
    expect(
      store.getState().terminal.workbenchOpenByChat["chat-a"],
    ).toBeUndefined();
  });
});
