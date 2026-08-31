import { describe, expect, test } from "vitest";

import {
  agentsPanelSlice,
  autoOpenRequested,
  panelClosed,
  panelOpened,
  selectAgentsPanelOpen,
  selectAgentsPanelTab,
  selectAgentsPanelUserClosed,
  tabChanged,
} from "./agentsPanelSlice";

describe("agentsPanelSlice", () => {
  test("remembers a manual close while allowing a manual reopen", () => {
    let state = agentsPanelSlice.reducer(undefined, panelClosed("chat-1"));
    state = agentsPanelSlice.reducer(state, autoOpenRequested("chat-1"));

    expect(selectAgentsPanelOpen({ agentsPanel: state }, "chat-1")).toBe(false);
    expect(selectAgentsPanelUserClosed({ agentsPanel: state }, "chat-1")).toBe(true);

    state = agentsPanelSlice.reducer(state, panelOpened("chat-1"));
    expect(selectAgentsPanelOpen({ agentsPanel: state }, "chat-1")).toBe(true);
    expect(selectAgentsPanelUserClosed({ agentsPanel: state }, "chat-1")).toBe(false);
  });

  test("auto-opens unclosed chats and changes tabs", () => {
    let state = agentsPanelSlice.reducer(undefined, autoOpenRequested("chat-1"));
    state = agentsPanelSlice.reducer(state, tabChanged("all"));

    expect(selectAgentsPanelOpen({ agentsPanel: state }, "chat-1")).toBe(true);
    expect(selectAgentsPanelTab({ agentsPanel: state })).toBe("all");
  });
});
