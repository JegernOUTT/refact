import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { render, screen, waitFor } from "../../utils/test-utils";
import { act } from "react-dom/test-utils";
import { ComposerToolbar } from "./ComposerToolbar";

function DraftControl() {
  const [draft, setDraft] = useState("");
  return (
    <input
      aria-label="Scope draft"
      value={draft}
      onChange={(event) => setDraft(event.target.value)}
    />
  );
}

function Harness() {
  const [menus, setMenus] = useState(0);
  return (
    <ComposerToolbar
      menuCount={menus}
      onOpenChange={(open) => setMenus((count) => count + (open ? 1 : -1))}
      items={[
        { id: "model", width: 160, content: <DraftControl /> },
        {
          id: "mode",
          width: 160,
          content: <button type="button">Mode</button>,
        },
        {
          id: "worktree",
          width: 120,
          content: <button type="button">Worktree</button>,
        },
      ]}
      send={<button type="button">Stop</button>}
    />
  );
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

test("moves whole controls into accessible More on resize without remounting state", async () => {
  let width = 1100;
  let measure: () => void = () => undefined;
  const disconnect = vi.fn();
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(
    () => width,
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(private callback: () => void) {}
      observe(target: Element) {
        if (target.getAttribute("data-testid") === "composer-toolbar")
          measure = this.callback;
      }
      disconnect = disconnect;
    },
  );
  const { user, unmount } = render(<Harness />);
  const draft = screen.getByRole("textbox", { name: "Scope draft" });
  await user.type(draft, "keep scope");
  await user.click(screen.getByRole("button", { name: "Stop" }));
  width = 280;
  act(() => measure());
  await waitFor(() =>
    expect(
      draft.closest("[data-composer-control]")?.parentElement,
    ).not.toHaveClass("inline"),
  );
  expect(
    screen.getByRole("button", { name: "More composer controls" }),
  ).toHaveAttribute("aria-expanded", "false");
  expect(screen.getByRole("button", { name: "Stop" })).toBeVisible();
  await user.click(
    screen.getByRole("button", { name: "More composer controls" }),
  );
  expect(screen.getByRole("textbox", { name: "Scope draft" })).toBe(draft);
  expect(draft).toHaveValue("keep scope");
  expect(screen.getByRole("button", { name: "Mode" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Worktree" })).toBeVisible();
  await user.keyboard("{Escape}");
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "More composer controls" }),
    ).toHaveAttribute("aria-expanded", "false"),
  );
  expect(
    screen.getByRole("button", { name: "More composer controls" }),
  ).toHaveFocus();
  width = 1100;
  act(() => measure());
  await waitFor(() =>
    expect(screen.getByRole("textbox", { name: "Scope draft" })).toHaveValue(
      "keep scope",
    ),
  );
  unmount();
  expect(disconnect).toHaveBeenCalled();
});
