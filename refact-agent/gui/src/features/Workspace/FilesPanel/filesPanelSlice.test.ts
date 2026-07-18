import { describe, expect, it } from "vitest";

import { setUpStore } from "../../../app/store";
import { openFileInFilesPanel } from "./filesPanelSlice";

describe("openFileInFilesPanel", () => {
  it("opens and focuses the Files tab while targeting the requested line", () => {
    const store = setUpStore();

    store.dispatch(
      openFileInFilesPanel({ path: "/workspace/src/main.ts", line: 12 }),
    );

    expect(store.getState().workspace.tabs).toContain("files:main");
    expect(store.getState().workspace.activeTabId).toBe("files:main");
    expect(store.getState().filesPanel.viewerTarget).toEqual({
      path: "/workspace/src/main.ts",
      line: 12,
    });
    expect(store.getState().filesPanel.expandedDirectories).toEqual([
      "/workspace",
      "/workspace/src",
    ]);
  });
});
