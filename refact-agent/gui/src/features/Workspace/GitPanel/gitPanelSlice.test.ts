import { describe, expect, it } from "vitest";

import { setUpStore } from "../../../app/store";
import { openGitFile, setActiveGitRoot } from "./gitPanelSlice";

describe("gitPanelSlice", () => {
  it("opens and focuses one main Git surface for selected files", () => {
    const store = setUpStore();

    store.dispatch(
      openGitFile({ root: "/repo", path: "src/app.ts", staged: false }),
    );
    store.dispatch(
      openGitFile({ root: "/repo", path: "src/lib.ts", staged: true }),
    );

    expect(store.getState().workspace.tabs).toEqual(["git:main"]);
    expect(store.getState().workspace.activeTabId).toBe("git:main");
    expect(store.getState().gitPanel.selectedFile).toEqual({
      root: "/repo",
      path: "src/lib.ts",
      staged: true,
    });
  });

  it("clears a selected file when the active root changes", () => {
    const store = setUpStore();
    store.dispatch(
      openGitFile({ root: "/repo", path: "src/app.ts", staged: false }),
    );

    store.dispatch(setActiveGitRoot("/other"));

    expect(store.getState().gitPanel).toEqual({
      activeRoot: "/other",
      selectedFile: null,
    });
  });
});
