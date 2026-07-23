import { describe, expect, it } from "vitest";
import { act } from "react-dom/test-utils";

import { fireEvent, render, screen } from "../../../utils/test-utils";
import statusDotStyles from "../../../components/ui/StatusDot/StatusDot.module.css";
import { sessionAdded } from "../TerminalPanel";
import {
  DRAWER_MIN_HEIGHT,
  clampDrawerHeight,
  setDrawerOpen,
} from "../workspaceSlice";
import { Drawer } from "./Drawer";

describe("Drawer", () => {
  it("renders collapsed session status dots and expands and collapses", async () => {
    const view = render(<Drawer>Terminal body</Drawer>);
    act(() => {
      view.store.dispatch(
        sessionAdded({
          process_id: "running-one",
          title: "zsh · running",
          status: "running",
        }),
      );
      view.store.dispatch(
        sessionAdded({
          process_id: "killed-three",
          title: "build · killed",
          status: "killed",
        }),
      );
      view.store.dispatch(
        sessionAdded({
          process_id: "failed-two",
          title: "tests · failed",
          status: "failed",
        }),
      );
    });

    expect(
      screen.getByRole("button", {
        name: "Expand terminal drawer, 3 sessions",
      }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("zsh · running: running")).toBeInTheDocument();
    expect(screen.getByLabelText("tests · failed: failed")).toBeInTheDocument();
    expect(screen.getByLabelText("build · killed: killed")).toHaveClass(
      statusDotStyles.danger,
      statusDotStyles.small,
    );
    expect(screen.getByLabelText("zsh · running: running")).toHaveClass(
      "rf-status-pulse",
    );

    await view.user.click(
      screen.getByRole("button", {
        name: "Expand terminal drawer, 3 sessions",
      }),
    );
    expect(view.store.getState().workspace.drawer?.open).toBe(true);
    expect(
      screen.getByRole("separator", { name: "Resize terminal drawer" }),
    ).toBeInTheDocument();

    await view.user.click(
      screen.getByRole("button", { name: "Collapse terminal drawer" }),
    );
    expect(view.store.getState().workspace.drawer?.open).toBe(false);
  });

  it("clamps drag height to the minimum and half of the viewport", () => {
    expect(clampDrawerHeight(1, 1000)).toBe(DRAWER_MIN_HEIGHT);
    expect(clampDrawerHeight(900, 1000)).toBe(500);

    const view = render(<Drawer>Terminal body</Drawer>);
    act(() => {
      view.store.dispatch(setDrawerOpen(true));
    });
    const drawer = screen.getByLabelText("Terminal drawer");
    drawer.getBoundingClientRect = () => ({ height: 280 }) as DOMRect;
    const separator = screen.getByRole("separator", {
      name: "Resize terminal drawer",
    });

    fireEvent.pointerDown(separator, { button: 0, clientY: 280 });
    fireEvent.pointerMove(window, { clientY: 1000 });
    fireEvent.pointerUp(window);

    expect(view.store.getState().workspace.drawer?.height).toBe(
      DRAWER_MIN_HEIGHT,
    );
  });
});
