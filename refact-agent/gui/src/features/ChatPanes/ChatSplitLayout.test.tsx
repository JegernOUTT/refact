import { act } from "react-dom/test-utils";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor } from "../../utils/test-utils";

import { render } from "../../utils/test-utils";
import { createChatWithId } from "../Chat/Thread";
import { hydratePaneLayout } from "./panesSlice";
import { ChatSplitLayout } from "./ChatSplitLayout";
import { findLeaf, type LeafPane, type SplitNode } from "./panesTree";
import styles from "./ChatSplitLayout.module.css";

vi.mock("./ChatPane", async () => {
  const React = await vi.importActual<typeof import("react")>("react");

  return {
    ChatPane: ({ leafId }: { leafId: string }) =>
      React.createElement(
        "div",
        { "data-testid": `mock-chat-pane-${leafId}` },
        `Pane ${leafId}`,
      ),
  };
});

function setMeasuredWidth(width: number) {
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(width);
}

function splitRoot(dir: SplitNode["dir"]): SplitNode {
  return {
    kind: "split",
    id: `root:split:${dir}`,
    dir,
    sizes: [0.5, 0.5],
    children: [
      {
        kind: "leaf",
        id: "root",
        tabIds: ["chat-a"],
        activeTabId: "chat-a",
      },
      {
        kind: "leaf",
        id: "right",
        tabIds: ["chat-b"],
        activeTabId: "chat-b",
      },
    ],
  };
}

function renderSplitLayout(dir: SplitNode["dir"]) {
  const view = render(<ChatSplitLayout />);

  act(() => {
    view.store.dispatch(
      createChatWithId({ id: "chat-a", title: "Chat Alpha", mode: "agent" }),
    );
    view.store.dispatch(
      createChatWithId({ id: "chat-b", title: "Chat Beta", mode: "agent" }),
    );
    view.store.dispatch(
      hydratePaneLayout({
        root: splitRoot(dir),
        focusedLeafId: "root",
      }),
    );
  });

  return view;
}

function assertLeaf(node: LeafPane | null): LeafPane {
  if (!node) {
    throw new Error("expected leaf");
  }

  return node;
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("ChatSplitLayout", () => {
  it("renders a row split with a vertical divider", async () => {
    setMeasuredWidth(1024);
    renderSplitLayout("row");

    await waitFor(() => {
      expect(screen.getByTestId("pane-vertical-divider")).toBeInTheDocument();
    });

    expect(screen.getByTestId("mock-chat-pane-root")).toBeInTheDocument();
    expect(screen.getByTestId("mock-chat-pane-right")).toBeInTheDocument();
    expect(
      screen.queryByTestId("pane-horizontal-divider"),
    ).not.toBeInTheDocument();
  });

  it("renders a col split with a horizontal divider", async () => {
    setMeasuredWidth(1024);
    renderSplitLayout("col");

    await waitFor(() => {
      expect(screen.getByTestId("pane-horizontal-divider")).toBeInTheDocument();
    });

    expect(screen.getByTestId("mock-chat-pane-root")).toBeInTheDocument();
    expect(screen.getByTestId("mock-chat-pane-right")).toBeInTheDocument();
    expect(
      screen.queryByTestId("pane-vertical-divider"),
    ).not.toBeInTheDocument();
  });

  it("uses stacked layout class for narrow containers", async () => {
    setMeasuredWidth(420);
    const view = renderSplitLayout("row");
    const layout = view.container.querySelector("[data-breakpoint]");

    await waitFor(() => {
      expect(layout).toHaveAttribute("data-breakpoint", "narrow");
    });

    expect(layout).toHaveClass(styles.stackedLayout);
    expect(
      screen.queryByTestId("pane-vertical-divider"),
    ).not.toBeInTheDocument();
  });

  it("splits the focused pane to the right from the toolbar", async () => {
    setMeasuredWidth(1024);
    const view = renderSplitLayout("row");

    act(() => {
      view.store.dispatch(
        createChatWithId({ id: "chat-a", title: "Chat Alpha", mode: "agent" }),
      );
      view.store.dispatch(
        createChatWithId({ id: "chat-b", title: "Chat Beta", mode: "agent" }),
      );
      view.store.dispatch(
        hydratePaneLayout({
          root: {
            kind: "leaf",
            id: "root",
            tabIds: ["chat-a", "chat-b"],
            activeTabId: "chat-b",
          },
          focusedLeafId: "root",
        }),
      );
    });

    await userEvent.click(screen.getByRole("button", { name: "Split Right" }));

    await waitFor(() => {
      const state = view.store.getState().panes;
      expect(state.root.kind).toBe("split");
      expect(state.focusedLeafId).toBe("root:sibling:chat-b");
    });

    const root = view.store.getState().panes.root;
    const originalLeaf = assertLeaf(findLeaf(root, "root"));
    const siblingLeaf = assertLeaf(findLeaf(root, "root:sibling:chat-b"));

    expect(originalLeaf.tabIds).toEqual(["chat-a"]);
    expect(siblingLeaf.tabIds).toEqual(["chat-b"]);
    expect(siblingLeaf.activeTabId).toBe("chat-b");
  });
});
