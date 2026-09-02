import { describe, expect, test } from "vitest";

import {
  setDockOpen,
  setDockSection,
  toggleDock,
} from "../Workspace/workspaceSlice";
import {
  agentsPanelSlice,
  agentsSectionUserClosed,
  agentsSectionUserOpened,
  autoOpenCleared,
  autoOpenRequested,
  selectAgentsPanelAutoOpenedFor,
  selectAgentsPanelTab,
  selectAgentsPanelUserClosed,
  tabChanged,
} from "./agentsPanelSlice";

const reduce = agentsPanelSlice.reducer;

describe("agentsPanelSlice", () => {
  test("starts on the active tab with nothing auto-opened", () => {
    const state = reduce(undefined, { type: "@@INIT" });

    expect(selectAgentsPanelTab({ agentsPanel: state })).toBe("active");
    expect(selectAgentsPanelAutoOpenedFor({ agentsPanel: state })).toBeNull();
    expect(selectAgentsPanelUserClosed({ agentsPanel: state }, "chat-1")).toBe(
      false,
    );
  });

  test("changes the tab", () => {
    let state = reduce(undefined, tabChanged("all"));
    expect(selectAgentsPanelTab({ agentsPanel: state })).toBe("all");

    state = reduce(state, tabChanged("active"));
    expect(selectAgentsPanelTab({ agentsPanel: state })).toBe("active");
  });

  test("records the chat an auto-open was requested for", () => {
    const state = reduce(undefined, autoOpenRequested("chat-1"));

    expect(selectAgentsPanelAutoOpenedFor({ agentsPanel: state })).toBe(
      "chat-1",
    );
  });

  test("suppresses auto-open for a chat the user closed, until they reopen it", () => {
    let state = reduce(undefined, agentsSectionUserClosed("chat-1"));
    state = reduce(state, autoOpenRequested("chat-1"));

    expect(selectAgentsPanelUserClosed({ agentsPanel: state }, "chat-1")).toBe(
      true,
    );
    expect(selectAgentsPanelAutoOpenedFor({ agentsPanel: state })).toBeNull();

    state = reduce(state, autoOpenRequested("chat-2"));
    expect(selectAgentsPanelAutoOpenedFor({ agentsPanel: state })).toBe(
      "chat-2",
    );

    state = reduce(state, agentsSectionUserOpened("chat-1"));
    expect(selectAgentsPanelUserClosed({ agentsPanel: state }, "chat-1")).toBe(
      false,
    );
    expect(selectAgentsPanelAutoOpenedFor({ agentsPanel: state })).toBeNull();

    state = reduce(state, autoOpenRequested("chat-1"));
    expect(selectAgentsPanelAutoOpenedFor({ agentsPanel: state })).toBe(
      "chat-1",
    );
  });

  test("clears the auto-open marker explicitly", () => {
    let state = reduce(undefined, autoOpenRequested("chat-1"));
    state = reduce(state, autoOpenCleared());

    expect(selectAgentsPanelAutoOpenedFor({ agentsPanel: state })).toBeNull();
    expect(selectAgentsPanelUserClosed({ agentsPanel: state }, "chat-1")).toBe(
      false,
    );
  });

  test.each([
    ["setDockSection", setDockSection("files")],
    ["setDockOpen", setDockOpen(false)],
    ["toggleDock", toggleDock()],
  ])(
    "drops the auto-open marker when the dock is driven by %s",
    (_, action) => {
      let state = reduce(undefined, autoOpenRequested("chat-1"));
      state = reduce(state, action);

      expect(selectAgentsPanelAutoOpenedFor({ agentsPanel: state })).toBeNull();
      expect(
        selectAgentsPanelUserClosed({ agentsPanel: state }, "chat-1"),
      ).toBe(false);
    },
  );

  test("keeps the tab selection across dock interactions", () => {
    let state = reduce(undefined, tabChanged("all"));
    state = reduce(state, autoOpenRequested("chat-1"));
    state = reduce(state, setDockSection("agents"));

    expect(selectAgentsPanelTab({ agentsPanel: state })).toBe("all");
    expect(selectAgentsPanelAutoOpenedFor({ agentsPanel: state })).toBeNull();
  });
});
