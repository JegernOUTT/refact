import type { ReactNode } from "react";
import { Provider } from "react-redux";
import { fireEvent, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { setUpStore } from "../../app/store";
import { updateConfig } from "../Config/configSlice";
import { useWorkspaceShortcuts } from "./useWorkspaceShortcuts";

function renderShortcuts() {
  const store = setUpStore();
  const wrapper = ({ children }: { children: ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  const view = renderHook(() => useWorkspaceShortcuts(), { wrapper });
  return { store, ...view };
}

describe("useWorkspaceShortcuts", () => {
  it("toggles web workspace chrome and selects visible dock sections", () => {
    const { store } = renderShortcuts();

    fireEvent.keyDown(window, { key: "b", ctrlKey: true });
    expect(store.getState().workspace.dock?.open).toBe(false);

    fireEvent.keyDown(window, { key: "B", metaKey: true });
    expect(store.getState().workspace.dock?.open).toBe(true);

    fireEvent.keyDown(window, { key: "j", ctrlKey: true });
    expect(store.getState().workspace.drawer?.open).toBe(true);

    fireEvent.keyDown(window, { key: "2", ctrlKey: true });
    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "git",
    });

    fireEvent.keyDown(window, { key: "3", metaKey: true });
    expect(store.getState().workspace.dock?.section).toBe("tasks");
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
    renderHook(() => useWorkspaceShortcuts(), { wrapper });

    fireEvent.keyDown(window, { key: "1", ctrlKey: true });
    expect(store.getState().workspace.dock?.section).toBe("files");
    fireEvent.keyDown(window, { key: "2", ctrlKey: true });
    expect(store.getState().workspace.dock?.section).toBe("git");
  });

  it("does not fire from editable controls, contenteditable regions, or xterm", () => {
    const { store } = renderShortcuts();
    const input = document.body.appendChild(document.createElement("input"));
    const editable = document.body.appendChild(document.createElement("div"));
    editable.contentEditable = "true";
    const terminal = document.body.appendChild(document.createElement("div"));
    terminal.className = "xterm";

    fireEvent.keyDown(input, { key: "b", ctrlKey: true });
    fireEvent.keyDown(editable, { key: "2", ctrlKey: true });
    fireEvent.keyDown(terminal, { key: "j", metaKey: true });

    expect(store.getState().workspace.dock).toMatchObject({
      open: true,
      section: "files",
    });
    expect(store.getState().workspace.drawer?.open).toBe(false);

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
    expect(store.getState().workspace.drawer?.open).toBe(false);
  });
});
