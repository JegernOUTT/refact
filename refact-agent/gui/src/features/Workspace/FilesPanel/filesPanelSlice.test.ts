import { describe, expect, it } from "vitest";

import { setUpStore } from "../../../app/store";
import { openFileInFilesPanel } from "./filesPanelSlice";

describe("openFileInFilesPanel", () => {
  it("opens and focuses a deduplicated file viewer tab", () => {
    const store = setUpStore();

    store.dispatch(
      openFileInFilesPanel({ path: "/workspace/src/main.ts", line: 12 }),
    );

    expect(store.getState().workspace.tabs).toContain(
      "file:/workspace/src/main.ts",
    );
    expect(store.getState().workspace.activeTabId).toBe(
      "file:/workspace/src/main.ts",
    );
    expect(store.getState().filesPanel.viewerTarget).toEqual({
      path: "/workspace/src/main.ts",
      line: 12,
    });
    expect(store.getState().filesPanel.expandedDirectories).toEqual([
      "/workspace",
      "/workspace/src",
    ]);

    store.dispatch(
      openFileInFilesPanel({ path: "/workspace/src/main.ts", line: 18 }),
    );
    expect(
      store
        .getState()
        .workspace.tabs.filter((tab) => tab === "file:/workspace/src/main.ts"),
    ).toHaveLength(1);
  });
});
