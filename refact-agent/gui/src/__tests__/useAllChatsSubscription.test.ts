import { describe, it, expect } from "vitest";
import { pickDesiredChatSubscriptions } from "../hooks/useAllChatsSubscription";
import {
  addSurfaceToPane,
  openTab,
  selectVisibleThreadIds,
  setActiveTab,
  splitTab,
  workspaceSlice,
} from "../features/Workspace";
import { makeSurfaceKey } from "../features/Workspace/surfaceKey";

describe("pickDesiredChatSubscriptions", () => {
  it("subscribes only chats visible on screen", () => {
    const result = pickDesiredChatSubscriptions({
      visibleThreadIds: ["chat-2", "chat-5", "chat-7"],
    });

    expect(result).toEqual(["chat-2", "chat-5", "chat-7"]);
  });

  it("deduplicates visible chats while preserving order", () => {
    const result = pickDesiredChatSubscriptions({
      visibleThreadIds: ["chat-2", "chat-5", "chat-2", "chat-7"],
    });

    expect(result).toEqual(["chat-2", "chat-5", "chat-7"]);
  });

  it("does not keep chat subscriptions while the page is hidden", () => {
    const result = pickDesiredChatSubscriptions({
      visibleThreadIds: ["chat-1", "chat-2"],
      documentVisible: false,
    });

    expect(result).toEqual([]);
  });

  it("subscribes visible chat surfaces from the active workspace group only", () => {
    const chatA = makeSurfaceKey("chat", "chat-a");
    const chatB = makeSurfaceKey("chat", "chat-b");
    const chatC = makeSurfaceKey("chat", "chat-c");
    let workspace = workspaceSlice.reducer(undefined, openTab(chatA));
    workspace = workspaceSlice.reducer(workspace, openTab(chatB));
    workspace = workspaceSlice.reducer(workspace, openTab(chatC));
    workspace = workspaceSlice.reducer(workspace, setActiveTab(chatA));
    workspace = workspaceSlice.reducer(
      workspace,
      splitTab({ tabId: chatA, dir: "row" }),
    );
    workspace = workspaceSlice.reducer(
      workspace,
      addSurfaceToPane({
        tabId: chatA,
        leafId: "root:sibling:chat:chat-a",
        surfaceKey: chatB,
      }),
    );

    const visibleThreadIds = selectVisibleThreadIds({ workspace });
    const result = pickDesiredChatSubscriptions({ visibleThreadIds });

    expect(result).toEqual(["chat-a", "chat-b"]);
  });
});
