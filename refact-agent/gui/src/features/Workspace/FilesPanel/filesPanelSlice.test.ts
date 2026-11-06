import { describe, expect, it } from "vitest";

import { setUpStore } from "../../../app/store";
import { createChatWithId } from "../../Chat/Thread";
import {
  openTab,
  splitTab,
  focusPane,
  closePaneTab,
  clearWorkspaceChatState,
  selectFocusedWorkspaceChatId,
  selectSurfaceChatId,
} from "../workspaceSlice";
import { collectTabIds } from "../../ChatPanes/panesTree";
import { makeSurfaceKey } from "../surfaceKey";
import {
  applyLiveFileUpdate,
  clearLiveFileUpdate,
  clearLiveFileUpdatesForChat,
  markLiveFileUpdateAuthoritative,
  openFileInFilesPanel,
  validateAbsoluteFilePath,
} from "./filesPanelSlice";

describe("openFileInFilesPanel", () => {
  it("does not assign an explicitly unbound file when reopened from another chat", () => {
    const store = setUpStore();
    const file = "file:/unbound.txt";
    store.dispatch(
      openFileInFilesPanel({ path: "/unbound.txt", manual: true }),
    );
    expect(selectSurfaceChatId(store.getState(), file)).toBeNull();
    store.dispatch(createChatWithId({ id: "later-chat" }));
    store.dispatch(openTab("chat:later-chat"));
    store.dispatch(
      openFileInFilesPanel({ path: "/unbound.txt", manual: true }),
    );
    expect(selectSurfaceChatId(store.getState(), file)).toBeNull();
    store.dispatch(clearWorkspaceChatState("later-chat"));
    expect(
      Object.values(store.getState().workspace.groups).flatMap((group) =>
        group ? collectTabIds(group.root) : [],
      ),
    ).toContain(file);
    expect(selectSurfaceChatId(store.getState(), file)).toBeNull();
  });

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

  it("opens new files in the focused leaf, activates existing locations, and keeps explicit new tabs in the same local layer", () => {
    const store = setUpStore();
    store.dispatch(createChatWithId({ id: "pane-chat", title: "Pane chat" }));
    store.dispatch(openTab("chat:pane-chat"));
    store.dispatch(openFileInFilesPanel({ path: "/a" }));
    expect(store.getState().workspace.tabs).toEqual(["chat:pane-chat"]);
    expect(
      store.getState().workspace.groups["chat:pane-chat"]?.root,
    ).toMatchObject({
      kind: "leaf",
      tabIds: ["chat:pane-chat", "file:/a"],
      activeTabId: "file:/a",
    });
    store.dispatch(splitTab({ tabId: "chat:pane-chat", dir: "row" }));
    const siblingId =
      store.getState().workspace.groups["chat:pane-chat"]?.focusedLeafId ??
      "missing";
    store.dispatch(openFileInFilesPanel({ path: "/b" }));
    expect(
      store.getState().workspace.groups["chat:pane-chat"]?.focusedLeafId,
    ).toBe(siblingId);
    store.dispatch(focusPane({ tabId: "chat:pane-chat", leafId: "root" }));
    store.dispatch(openFileInFilesPanel({ path: "/b", line: 5 }));
    expect(
      store.getState().workspace.groups["chat:pane-chat"]?.focusedLeafId,
    ).toBe(siblingId);
    store.dispatch(openFileInFilesPanel({ path: "/b", openInNewTab: true }));
    expect(store.getState().workspace.tabs).toEqual(["chat:pane-chat"]);
    store.dispatch(openFileInFilesPanel({ path: "/c" }));
    const group = store.getState().workspace.groups["chat:pane-chat"];
    expect(group?.focusedLeafId).toBe(siblingId);
    store.dispatch(openFileInFilesPanel({ path: "/b", openInNewTab: true }));
    expect(store.getState().workspace.tabs).toEqual(["chat:pane-chat"]);
    expect(
      store.getState().workspace.groups["chat:pane-chat"]?.root,
    ).toMatchObject({ kind: "split" });
    store.dispatch(
      closePaneTab({
        tabId: "chat:pane-chat",
        leafId: "root",
        surfaceKey: "chat:pane-chat",
      }),
    );
    expect(store.getState().chat.open_thread_ids).not.toContain("pane-chat");
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

    expect(
      selectSurfaceChatId(
        store.getState(),
        "file:/worktrees/chat-b/src/main.ts",
      ),
    ).toBe("chat-b");
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

describe("manual absolute path targets", () => {
  it("validates absolute file paths", () => {
    expect(validateAbsoluteFilePath("  /tmp/a.txt ")).toEqual({
      ok: true,
      path: "/tmp/a.txt",
    });
    expect(validateAbsoluteFilePath("C:\\Users\\a.txt")).toMatchObject({
      ok: true,
    });
    expect(validateAbsoluteFilePath("\\\\server\\share\\a.txt")).toMatchObject({
      ok: true,
    });
    expect(validateAbsoluteFilePath("")).toMatchObject({ ok: false });
    expect(validateAbsoluteFilePath("src/a.ts")).toMatchObject({ ok: false });
    expect(validateAbsoluteFilePath("~/a.ts")).toMatchObject({ ok: false });
    expect(validateAbsoluteFilePath("/tmp/dir/")).toMatchObject({ ok: false });
    expect(validateAbsoluteFilePath("/tmp/a\nb")).toMatchObject({ ok: false });
    expect(validateAbsoluteFilePath(`/${"a".repeat(5000)}`)).toMatchObject({
      ok: false,
    });
  });

  it("dedups the manual tab and keeps the manual flag on reopen", () => {
    const store = setUpStore({
      current_project: { name: "workspace", workspaceRoots: ["/workspace"] },
    });

    store.dispatch(openFileInFilesPanel({ path: "/etc/hosts", manual: true }));
    store.dispatch(openFileInFilesPanel({ path: "/etc/hosts", line: 3 }));

    expect(
      store
        .getState()
        .workspace.tabs.filter((tab) => tab === "file:/etc/hosts"),
    ).toHaveLength(1);
    expect(store.getState().filesPanel.viewerTargets["/etc/hosts"]).toEqual({
      path: "/etc/hosts",
      line: 3,
      manual: true,
    });
    expect(store.getState().filesPanel.expandedDirectories).toEqual([]);
    expect(store.getState().current_project.workspaceRoots).toEqual([
      "/workspace",
    ]);
  });
});
