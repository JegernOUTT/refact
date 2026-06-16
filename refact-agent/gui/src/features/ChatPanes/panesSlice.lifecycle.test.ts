import { configureStore } from "@reduxjs/toolkit";
import type { UnknownAction } from "@reduxjs/toolkit";
import { describe, expect, it } from "vitest";

import { closeThread, createChatWithId } from "../Chat/Thread/actions";
import { chatReducer } from "../Chat/Thread/reducer";
import { findLeaf, type LeafPane, type PaneNode } from "./panesTree";
import {
  addTabToFocusedPane,
  hydratePaneLayout,
  panesSlice,
  reconcilePanesWithOpenThreads,
  removeTabEverywhere,
  splitPane,
} from "./panesSlice";

const paneReducer = panesSlice.reducer;

const leaf = (
  id: string,
  tabIds: string[] = [],
  activeTabId: string | null = tabIds[0] ?? null,
): LeafPane => ({
  kind: "leaf",
  id,
  tabIds,
  activeTabId,
});

function reduceWithPaneInvariant(
  state:
    | {
        chat: ReturnType<typeof chatReducer>;
        panes: ReturnType<typeof paneReducer>;
      }
    | undefined,
  action: UnknownAction,
) {
  const nextState = {
    chat: chatReducer(state?.chat, action),
    panes: paneReducer(state?.panes, action),
  };

  return {
    ...nextState,
    panes: reconcilePanesWithOpenThreads(
      nextState.panes,
      nextState.chat.open_thread_ids,
      nextState.chat.current_thread_id,
    ),
  };
}

describe("pane lifecycle", () => {
  it("removing a closed tab collapses an empty non-root leaf", () => {
    const root: PaneNode = {
      kind: "split",
      id: "root:split:row",
      dir: "row",
      sizes: [0.25, 0.75],
      children: [
        leaf("left", ["chat-a"], "chat-a"),
        leaf("right", ["chat-b"], "chat-b"),
      ],
    };

    const state = paneReducer(
      { root, focusedLeafId: "right" },
      removeTabEverywhere("chat-b"),
    );

    expect(state.root).toEqual(leaf("left", ["chat-a"], "chat-a"));
    expect(state.focusedLeafId).toBe("left");
  });

  it("closeThread prunes the closed chat and collapses the empty split leaf", () => {
    const store = configureStore({ reducer: reduceWithPaneInvariant });

    store.dispatch(createChatWithId({ id: "chat-a", mode: "agent" }));
    store.dispatch(createChatWithId({ id: "chat-b", mode: "agent" }));
    store.dispatch(
      hydratePaneLayout({
        root: {
          kind: "split",
          id: "root:split:row",
          dir: "row",
          sizes: [0.4, 0.6],
          children: [
            leaf("left", ["chat-a"], "chat-a"),
            leaf("right", ["chat-b"], "chat-b"),
          ],
        },
        focusedLeafId: "right",
      }),
    );

    store.dispatch(closeThread({ id: "chat-b", force: true }));

    expect(store.getState().chat.open_thread_ids).toEqual(["chat-a"]);
    expect(store.getState().panes.root).toEqual(
      leaf("left", ["chat-a"], "chat-a"),
    );
    expect(store.getState().panes.focusedLeafId).toBe("left");
    expect(findLeaf(store.getState().panes.root, "right")).toBeNull();
  });

  it("closeThread degrades to a single empty root leaf after the last tab closes", () => {
    const store = configureStore({ reducer: reduceWithPaneInvariant });

    store.dispatch(createChatWithId({ id: "chat-a", mode: "agent" }));
    store.dispatch(addTabToFocusedPane("chat-a"));
    store.dispatch(splitPane({ leafId: "root", dir: "row", tabId: "chat-a" }));

    store.dispatch(closeThread({ id: "chat-a", force: true }));

    expect(store.getState().chat.open_thread_ids).toEqual([]);
    expect(store.getState().panes.root).toEqual(leaf("root", [], null));
    expect(store.getState().panes.focusedLeafId).toBe("root");
  });
});
