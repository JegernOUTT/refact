import { describe, expect, it } from "vitest";

import { setUpStore } from "../../../app/store";
import { openFileInFilesPanel } from "./filesPanelSlice";

describe("openFileInFilesPanel", () => {
  it("opens and focuses a deduplicated file viewer tab", () => {
    const store = setUpStore({
      current_project: {
        name: "workspace",
        workspaceRoots: ["/workspace"],
      },
    });

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

  it("expands only ancestors at or below a deep workspace root", () => {
    const store = setUpStore({
      current_project: {
        name: "engine",
        workspaceRoots: ["/w/a/b/engine"],
      },
    });

    store.dispatch(openFileInFilesPanel({ path: "/w/a/b/engine/src/x.rs" }));

    expect(store.getState().filesPanel.expandedDirectories).toEqual([
      "/w/a/b/engine",
      "/w/a/b/engine/src",
    ]);
  });

  it("clamps expansion to the configured root containing the file", () => {
    const store = setUpStore({
      current_project: {
        name: "workspace",
        workspaceRoots: ["/w/first", "/w/other/deep"],
      },
    });

    store.dispatch(openFileInFilesPanel({ path: "/w/other/deep/src/x.rs" }));

    expect(store.getState().filesPanel.expandedDirectories).toEqual([
      "/w/other/deep",
      "/w/other/deep/src",
    ]);
  });

  it("does not expand directories for a file outside configured roots", () => {
    const store = setUpStore({
      current_project: {
        name: "workspace",
        workspaceRoots: ["/w/project"],
      },
    });

    store.dispatch(openFileInFilesPanel({ path: "/outside/src/x.rs" }));

    expect(store.getState().filesPanel.expandedDirectories).toEqual([]);
  });
});
