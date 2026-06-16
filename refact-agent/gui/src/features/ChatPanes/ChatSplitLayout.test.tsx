import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor } from "../../utils/test-utils";

import { render } from "../../utils/test-utils";
import { hydratePaneLayout } from "./panesSlice";
import { ChatSplitLayout } from "./ChatSplitLayout";
import type { SplitNode } from "./panesTree";
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
      hydratePaneLayout({
        root: splitRoot(dir),
        focusedLeafId: "root",
      }),
    );
  });

  return view;
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
});
