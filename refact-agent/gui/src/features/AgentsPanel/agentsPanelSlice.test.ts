import { describe, expect, test } from "vitest";

import {
  setDockOpen,
  setDockSection,
  toggleDock,
} from "../Workspace/workspaceSlice";
import {
  agentsPanelSlice,
  selectAgentsPanelTab,
  tabChanged,
} from "./agentsPanelSlice";

const reduce = agentsPanelSlice.reducer;

describe("agentsPanelSlice", () => {
  test("starts on the active tab", () => {
    const state = reduce(undefined, { type: "@@INIT" });

    expect(selectAgentsPanelTab({ agentsPanel: state })).toBe("active");
  });

  test("changes the tab", () => {
    let state = reduce(undefined, tabChanged("all"));
    expect(selectAgentsPanelTab({ agentsPanel: state })).toBe("all");

    state = reduce(state, tabChanged("active"));
    expect(selectAgentsPanelTab({ agentsPanel: state })).toBe("active");
  });

  test.each([
    ["setDockSection", setDockSection("files")],
    ["setDockOpen", setDockOpen(false)],
    ["toggleDock", toggleDock()],
  ])("keeps the tab selection when the dock is driven by %s", (_, action) => {
    let state = reduce(undefined, tabChanged("all"));
    state = reduce(state, action);

    expect(selectAgentsPanelTab({ agentsPanel: state })).toBe("all");
  });
});
