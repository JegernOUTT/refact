import { describe, expect, it } from "vitest";

import { setUpStore } from "../../../app/store";
import { createChatWithId } from "../../Chat/Thread";
import { openTab, selectFocusedWorkspaceChatId } from "../workspaceSlice";
import { makeSurfaceKey } from "../surfaceKey";
import {
  applyLiveFileUpdate,
  clearLiveFileUpdate,
  clearLiveFileUpdatesForChat,
  markLiveFileUpdateAuthoritative,
  openFileInFilesPanel,
} from "./filesPanelSlice";

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

  it("captures chat and worktree context before file focus changes", () => {
    const store = setUpStore({
      current_project: {
        name: "workspace",
        workspaceRoots: ["/project"],
      },
    });
    store.dispatch(createChatWithId({ id: "chat-a", title: "Chat A" }));
    store.dispatch(
      createChatWithId({
        id: "chat-b",
        title: "Chat B",
        worktree: {
          id: "worktree-b",
          kind: "chat",
          root: "/worktrees/chat-b",
          source_workspace_root: "/project",
          repo_root: "/project",
          enforce: true,
        },
      }),
    );
    store.dispatch(openTab(makeSurfaceKey("chat", "chat-b")));

    store.dispatch(
      openFileInFilesPanel({ path: "/worktrees/chat-b/src/main.ts" }),
    );

    const fileSurface = makeSurfaceKey("file", "/worktrees/chat-b/src/main.ts");
    expect(store.getState().workspace.contextChatByTab).toEqual({
      [fileSurface]: "chat-b",
    });
    expect(selectFocusedWorkspaceChatId(store.getState())).toBe("chat-b");
    expect(store.getState().filesPanel.expandedDirectories).toEqual([
      "/worktrees/chat-b",
      "/worktrees/chat-b/src",
    ]);
  });
});

describe("live file updates", () => {
  const chunk = {
    file_name: "/workspace/src/main.ts",
    file_action: "edit",
    line1: 1,
    line2: 2,
    lines_remove: "old\n",
    lines_add: "new\n",
  };

  it("keeps the newest revision and rejects stale edits", () => {
    const store = setUpStore();
    store.dispatch(
      applyLiveFileUpdate({
        chatId: "chat-a",
        path: chunk.file_name,
        update: { revision: "10", chunks: [chunk], operation: "write" },
      }),
    );
    store.dispatch(
      applyLiveFileUpdate({
        chatId: "chat-a",
        path: chunk.file_name,
        update: {
          revision: "9",
          chunks: [{ ...chunk, lines_add: "stale\n" }],
          operation: "write",
        },
      }),
    );

    expect(
      store.getState().filesPanel.liveUpdatesByChat["chat-a"]?.[
        chunk.file_name
      ],
    ).toEqual({
      revision: "10",
      chunks: [chunk],
      operation: "write",
      authoritative: false,
    });
  });

  it("isolates the same path by chat and accepts only the latest reread", () => {
    const store = setUpStore();
    store.dispatch(
      applyLiveFileUpdate({
        chatId: "chat-a",
        path: chunk.file_name,
        update: { revision: "10", chunks: [chunk], operation: "write" },
      }),
    );
    store.dispatch(
      applyLiveFileUpdate({
        chatId: "chat-b",
        path: chunk.file_name,
        update: { revision: "4", chunks: [chunk], operation: "write" },
      }),
    );
    store.dispatch(
      applyLiveFileUpdate({
        chatId: "chat-a",
        path: chunk.file_name,
        update: { revision: "11", chunks: [chunk], operation: "write" },
      }),
    );
    store.dispatch(
      markLiveFileUpdateAuthoritative({
        chatId: "chat-a",
        path: chunk.file_name,
        revision: "10",
      }),
    );
    store.dispatch(
      markLiveFileUpdateAuthoritative({
        chatId: "chat-a",
        path: chunk.file_name,
        revision: "11",
      }),
    );

    expect(
      store.getState().filesPanel.liveUpdatesByChat["chat-a"]?.[
        chunk.file_name
      ],
    ).toMatchObject({ revision: "11", authoritative: true });
    expect(
      store.getState().filesPanel.liveUpdatesByChat["chat-b"]?.[
        chunk.file_name
      ],
    ).toMatchObject({ revision: "4", authoritative: false });
  });

  it("cleans a path conditionally and clears a closed chat", () => {
    const store = setUpStore();
    store.dispatch(
      applyLiveFileUpdate({
        chatId: "chat-a",
        path: chunk.file_name,
        update: { revision: "12", chunks: [chunk], operation: "write" },
      }),
    );
    store.dispatch(
      clearLiveFileUpdate({
        chatId: "chat-a",
        path: chunk.file_name,
        revision: "11",
      }),
    );
    expect(
      store.getState().filesPanel.liveUpdatesByChat["chat-a"]?.[
        chunk.file_name
      ],
    ).toBeDefined();

    store.dispatch(clearLiveFileUpdatesForChat("chat-a"));
    expect(
      store.getState().filesPanel.liveUpdatesByChat["chat-a"],
    ).toBeUndefined();
  });
});
